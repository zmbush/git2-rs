use std::ffi::CString;
use std::io::Write;
use std::mem;
use std::path::Path;
use std::process::{Command, Stdio};
use url::{self, UrlParser};

use {raw, Error, Config, IntoCString};
use util::Binding;

/// A structure to represent git credentials in libgit2.
pub struct Cred {
    raw: *mut raw::git_cred,
}

/// Management of the gitcredentials(7) interface.
pub struct CredentialHelper {
    /// A public field representing the currently discovered username from
    /// configuration.
    pub username: Option<String>,
    protocol: Option<String>,
    host: Option<String>,
    url: String,
    commands: Vec<String>,
}

impl Cred {
    /// Create a "default" credential usable for Negotiate mechanisms like NTLM
    /// or Kerberos authentication.
    pub fn default() -> Result<Cred, Error> {
        ::init();
        let mut out = 0 as *mut raw::git_cred;
        unsafe {
            try_call!(raw::git_cred_default_new(&mut out));
            Ok(Binding::from_raw(out))
        }
    }

    /// Create a new ssh key credential object used for querying an ssh-agent.
    ///
    /// The username specified is the username to authenticate.
    pub fn ssh_key_from_agent(username: &str) -> Result<Cred, Error> {
        ::init();
        let mut out = 0 as *mut raw::git_cred;
        let username = try!(CString::new(username));
        unsafe {
            try_call!(raw::git_cred_ssh_key_from_agent(&mut out, username));
            Ok(Binding::from_raw(out))
        }
    }

    /// Create a new passphrase-protected ssh key credential object.
    pub fn ssh_key(username: &str,
                   publickey: Option<&Path>,
                   privatekey: &Path,
                   passphrase: Option<&str>) -> Result<Cred, Error> {
        ::init();
        let username = try!(CString::new(username));
        let publickey = try!(::opt_cstr(publickey));
        let privatekey = try!(privatekey.into_c_string());
        let passphrase = try!(::opt_cstr(passphrase));
        let mut out = 0 as *mut raw::git_cred;
        unsafe {
            try_call!(raw::git_cred_ssh_key_new(&mut out, username, publickey,
                                                privatekey, passphrase));
            Ok(Binding::from_raw(out))
        }
    }

    /// Create a new plain-text username and password credential object.
    pub fn userpass_plaintext(username: &str,
                              password: &str) -> Result<Cred, Error> {
        ::init();
        let username = try!(CString::new(username));
        let password = try!(CString::new(password));
        let mut out = 0 as *mut raw::git_cred;
        unsafe {
            try_call!(raw::git_cred_userpass_plaintext_new(&mut out, username,
                                                           password));
            Ok(Binding::from_raw(out))
        }
    }

    /// Attempt to read `credential.helper` according to gitcredentials(7) [1]
    ///
    /// This function will attempt to parse the user's `credential.helper`
    /// configuration, invoke the necessary processes, and read off what the
    /// username/password should be for a particular url.
    ///
    /// The returned credential type will be a username/password credential if
    /// successful.
    ///
    /// [1]: https://www.kernel.org/pub/software/scm/git/docs/gitcredentials.html
    pub fn credential_helper(config: &Config,
                             url: &str,
                             username: Option<&str>)
                             -> Result<Cred, Error> {
        match CredentialHelper::new(url).config(config).username(username)
                               .execute() {
            Some((username, password)) => {
                Cred::userpass_plaintext(username.as_slice(),
                                         password.as_slice())
            }
            None => Err(Error::from_str("failed to acquire username/password \
                                         from local configuration"))
        }
    }

    /// Check whether a credential object contains username information.
    pub fn has_username(&self) -> bool {
        unsafe { raw::git_cred_has_username(self.raw) == 1 }
    }

    /// Return the type of credentials that this object represents.
    pub fn credtype(&self) -> raw::git_credtype_t {
        unsafe { (*self.raw).credtype }
    }

    /// Unwrap access to the underlying raw pointer, canceling the destructor
    pub unsafe fn unwrap(mut self) -> *mut raw::git_cred {
        mem::replace(&mut self.raw, 0 as *mut raw::git_cred)
    }
}

impl Binding for Cred {
    type Raw = *mut raw::git_cred;

    unsafe fn from_raw(raw: *mut raw::git_cred) -> Cred {
        Cred { raw: raw }
    }
    fn raw(&self) -> *mut raw::git_cred { self.raw }
}

impl Drop for Cred {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { ((*self.raw).free)(self.raw) }
        }
    }
}

impl CredentialHelper {
    /// Create a new credential helper object which will be used to probe git's
    /// local credential configuration.
    ///
    /// The url specified is the namespace on which this will query credentials.
    /// Invalid urls are currently ignored.
    pub fn new(url: &str) -> CredentialHelper {
        let mut ret = CredentialHelper {
            protocol: None,
            host: None,
            username: None,
            url: url.to_string(),
            commands: Vec::new(),
        };

        // Parse out the (protocol, host) if one is available
        let parsed_url = UrlParser::new().scheme_type_mapper(mapper).parse(url);
        match parsed_url {
            Ok(url) => {
                match url.host() {
                    Some(&url::Host::Domain(ref s)) => ret.host = Some(s.clone()),
                    _ => {}
                }
                ret.protocol = Some(url.scheme)
            }
            Err(..) => {}
        };
        return ret;

        fn mapper(s: &str) -> url::SchemeType {
            match s {
                "git" => url::SchemeType::Relative(9418),
                "ssh" => url::SchemeType::Relative(22),
                s => url::whatwg_scheme_type_mapper(s),
            }
        }
    }

    /// Set the username that this credential helper will query with.
    ///
    /// By default the username is `None`.
    pub fn username(&mut self, username: Option<&str>) -> &mut CredentialHelper {
        self.username = username.map(|s| s.to_string());
        self
    }

    /// Query the specified configuration object to discover commands to
    /// execute, usernames to query, etc.
    pub fn config(&mut self, config: &Config) -> &mut CredentialHelper {
        // Figure out the configured username/helper program.
        //
        // see http://git-scm.com/docs/gitcredentials.html#_configuration_options
        //
        // TODO: implement useHttpPath
        if self.username.is_none() {
            self.config_username(config);
        }
        self.config_helper(config);
        self
    }

    // Configure the queried username from `config`
    fn config_username(&mut self, config: &Config) {
        let key = self.exact_key("username");
        self.username = config.get_str(key.as_slice()).ok().or_else(|| {
            self.url_key("username").and_then(|s| {
                config.get_str(s.as_slice()).ok()
            })
        }).or_else(|| {
            config.get_str("credential.username").ok()
        }).map(|s| s.to_string());
    }

    // Discover all `helper` directives from `config`
    fn config_helper(&mut self, config: &Config) {
        let exact = config.get_str(self.exact_key("helper").as_slice());
        self.add_command(exact.ok());
        match self.url_key("helper") {
            Some(key) => {
                let url = config.get_str(key.as_slice());
                self.add_command(url.ok());
            }
            None => {}
        }
        let global = config.get_str("credential.helper");
        self.add_command(global.ok());
    }

    // Add a `helper` configured command to the list of commands to execute.
    //
    // see https://www.kernel.org/pub/software/scm/git/docs/technical
    //                           /api-credentials.html#_credential_helpers
    fn add_command(&mut self, cmd: Option<&str>) {
        let cmd = match cmd { Some(s) => s, None => return };
        if cmd.starts_with("!") {
            self.commands.push(cmd[1..].to_string());
        } else if cmd.starts_with("/") || cmd.starts_with("\\") ||
                  cmd[1..].starts_with(":\\") {
            self.commands.push(format!("\"{}\"", cmd));
        } else {
            self.commands.push(format!("git credential-{}", cmd));
        }
    }

    fn exact_key(&self, name: &str) -> String {
        format!("credential.{}.{}", self.url, name)
    }

    fn url_key(&self, name: &str) -> Option<String> {
        match (&self.host, &self.protocol) {
            (&Some(ref host), &Some(ref protocol)) => {
                Some(format!("credential.{}://{}.{}", protocol, host, name))
            }
            _ => None
        }
    }

    /// Execute this helper, attempting to discover a username/password pair.
    ///
    /// All I/O errors are ignored, (to match git behavior), and this function
    /// only succeeds if both a username and a password were found
    pub fn execute(&self) -> Option<(String, String)> {
        let mut username = self.username.clone();
        let mut password = None;
        for cmd in self.commands.iter() {
            let (u, p) = self.execute_cmd(cmd.as_slice(), &username);
            if u.is_some() && username.is_none() {
                username = u;
            }
            if p.is_some() && password.is_none() {
                password = p;
            }
            if username.is_some() && password.is_some() { break }
        }

        match (username, password) {
            (Some(u), Some(p)) => Some((u, p)),
            _ => None,
        }
    }

    // Execute the given `cmd`, providing the appropriate variables on stdin and
    // then afterwards parsing the output into the username/password on stdout.
    fn execute_cmd(&self, cmd: &str, username: &Option<String>)
                   -> (Option<String>, Option<String>) {
        macro_rules! my_try( ($e:expr) => (
            match $e { Ok(e) => e, Err(..) => return (None, None) }
        ) );

        let mut p = my_try!(Command::new("sh").arg("-c")
                                              .arg(&format!("{} get", cmd))
                                              .stdin(Stdio::capture())
                                              .stdout(Stdio::capture())
                                              .stderr(Stdio::capture())
                                              .spawn());
        // Ignore write errors as the command may not actually be listening for
        // stdin
        {
            let stdin = p.stdin.as_mut().unwrap();
            match self.protocol {
                Some(ref p) => { let _ = writeln!(stdin, "protocol={}", p); }
                None => {}
            }
            match self.host {
                Some(ref p) => { let _ = writeln!(stdin, "host={}", p); }
                None => {}
            }
            match *username {
                Some(ref p) => { let _ = writeln!(stdin, "username={}", p); }
                None => {}
            }
        }
        let output = my_try!(p.wait_with_output());
        if !output.status.success() { return (None, None) }
        return self.parse_output(output.stdout)
    }

    // Parse the output of a command into the username/password found
    fn parse_output(&self, output: Vec<u8>) -> (Option<String>, Option<String>) {
        // Parse the output of the command, looking for username/password
        let mut username = None;
        let mut password = None;
        for line in output.split(|t| *t == b'\n') {
            let mut parts = line.splitn(1, |t| *t == b'=');
            let key = parts.next().unwrap();
            let value = match parts.next() { Some(s) => s, None => continue };
            let value = match String::from_utf8(value.to_vec()) {
                Ok(s) => s,
                Err(..) => continue,
            };
            match key {
                b"username" => username = Some(value),
                b"password" => password = Some(value),
                _ => {}
            }
        }
        (username, password)
    }
}

#[cfg(test)]
mod test {
    use std::env;
    use std::fs::{self, File, TempDir};
    use std::io::prelude::*;
    use std::path::Path;

    use {Cred, Config, CredentialHelper, ConfigLevel};

    macro_rules! cfg( ($($k:expr => $v:expr),*) => ({
        let td = TempDir::new("git2-rs").unwrap();
        let mut cfg = Config::new().unwrap();
        cfg.add_file(&td.path().join("cfg"), ConfigLevel::Highest, false).unwrap();
        $(cfg.set_str($k, $v).unwrap();)*
        cfg
    }) );

    #[test]
    fn smoke() {
        Cred::default().unwrap();
    }

    #[test]
    fn credential_helper1() {
        let cfg = cfg! {
            "credential.helper" => "!f() { echo username=a; echo password=b; }; f"
        };
        let (u, p) = CredentialHelper::new("https://example.com/foo/bar")
                                      .config(&cfg)
                                      .execute().unwrap();
        assert_eq!(u.as_slice(), "a");
        assert_eq!(p.as_slice(), "b");
    }

    #[test]
    fn credential_helper2() {
        let cfg = cfg! {};
        assert!(CredentialHelper::new("https://example.com/foo/bar")
                                 .config(&cfg)
                                 .execute().is_none());
    }

    #[test]
    fn credential_helper3() {
        let cfg = cfg! {
            "credential.https://example.com.helper" =>
                    "!f() { echo username=c; }; f",
            "credential.helper" => "!f() { echo username=a; echo password=b; }; f"
        };
        let (u, p) = CredentialHelper::new("https://example.com/foo/bar")
                                      .config(&cfg)
                                      .execute().unwrap();
        assert_eq!(u.as_slice(), "c");
        assert_eq!(p.as_slice(), "b");
    }

    #[test]
    fn credential_helper4() {
        let td = TempDir::new("git2-rs").unwrap();
        let path = td.path().join("script");
        File::create(&path).unwrap().write(br"\
#!/bin/sh
echo username=c
").unwrap();
        chmod(&path);
        let cfg = cfg! {
            "credential.https://example.com.helper" =>
                    path.display().to_string().as_slice(),
            "credential.helper" => "!f() { echo username=a; echo password=b; }; f"
        };
        let (u, p) = CredentialHelper::new("https://example.com/foo/bar")
                                      .config(&cfg)
                                      .execute().unwrap();
        assert_eq!(u.as_slice(), "c");
        assert_eq!(p.as_slice(), "b");
    }

    #[test]
    fn credential_helper5() {
        let td = TempDir::new("git2-rs").unwrap();
        let path = td.path().join("git-credential-script");
        File::create(&path).unwrap().write(br"\
#!/bin/sh
echo username=c
").unwrap();
        chmod(&path);

        let paths = env::var("PATH").unwrap();
        let paths = env::split_paths(&paths)
                        .chain(path.parent().map(|p| p.to_path_buf()).into_iter());
        env::set_var("PATH", &env::join_paths(paths).unwrap());

        let cfg = cfg! {
            "credential.https://example.com.helper" => "script",
            "credential.helper" => "!f() { echo username=a; echo password=b; }; f"
        };
        let (u, p) = CredentialHelper::new("https://example.com/foo/bar")
                                      .config(&cfg)
                                      .execute().unwrap();
        assert_eq!(u.as_slice(), "c");
        assert_eq!(p.as_slice(), "b");
    }

    #[cfg(unix)]
    fn chmod(path: &Path) {
        use std::os::unix::prelude::*;
        let mut perms = path.metadata().unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
    #[cfg(windows)]
    fn chmod(_path: &Path) {}
}
