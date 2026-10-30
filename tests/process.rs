mod common;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Fixture {
    directory: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = owlytics::auth::random_token().unwrap();
        let directory =
            std::env::temp_dir().join(format!("owlytics-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        Self {
            directory,
            child: None,
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.join("nested/analytics.sqlite")
    }

    fn command(&self, hash: Option<&str>) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_owlytics"));
        command
            .env("OWLYTICS_BIND", "127.0.0.1:0")
            .env("OWLYTICS_DATABASE", self.database())
            .env("OWLYTICS_URL", "http://localhost:3000")
            .env("OWLYTICS_TRUSTED_PROXIES", "")
            .env_remove("OWLYTICS_PASSWORD_HASH")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(hash) = hash {
            command.env("OWLYTICS_PASSWORD_HASH", hash);
        }
        command
    }

    fn boot(&mut self) -> String {
        self.boot_with_hash(Some(&common::config().password_hash))
    }

    fn boot_with_hash(&mut self, hash: Option<&str>) -> String {
        assert!(self.child.is_none());
        let mut child = self.command(hash).spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        self.child = Some(child);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line);
            let _ = sender.send(result.map(|_| line));
        });
        let line = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("startup deadline exceeded")
            .unwrap();
        line.trim()
            .strip_prefix("Owlytics listening on http://")
            .expect("missing startup address")
            .to_owned()
    }

    fn startup_error(&mut self, hash: Option<&str>) -> String {
        assert!(self.child.is_none());
        self.child = Some(self.command(hash).stderr(Stdio::piped()).spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "server started without a usable credential"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = self.child.take().unwrap().wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty(), "must fail before listening");
        String::from_utf8(output.stderr).unwrap()
    }

    fn reset_password(&self, password: &str, hash: Option<&str>) -> std::process::Output {
        let mut child = self
            .command(hash)
            .arg("reset-password")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(password.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn healthcheck(address: &str) {
    let mut stream =
        TcpStream::connect_timeout(&address.parse().unwrap(), Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(b"GET /up HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.ends_with("\r\n\r\nok"), "{response}");
}

#[test]
fn binary_creates_persistent_database_and_recovers_after_process_stop() {
    let mut fixture = Fixture::new();
    let first_address = fixture.boot();
    healthcheck(&first_address);
    fixture.stop();
    let database = fixture.directory.join("nested/analytics.sqlite");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(database.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TABLE persistence_probe(value TEXT); INSERT INTO persistence_probe VALUES ('survives restart');").unwrap();
    drop(connection);
    let second_address = fixture.boot();
    healthcheck(&second_address);
    fixture.stop();
    let connection = rusqlite::Connection::open(database).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM persistence_probe", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "survives restart");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn startup_requires_a_valid_environment_hash_without_stored_credentials() {
    let unsupported = common::hash().replace("m=7168", "m=1048576");
    for existing_database in [false, true] {
        for (hash, message) in [
            (None, "set OWLYTICS_PASSWORD_HASH"),
            (Some(""), "set OWLYTICS_PASSWORD_HASH"),
            (Some("invalid-hash"), "OWLYTICS_PASSWORD_HASH is invalid"),
            (
                Some(unsupported.as_str()),
                "OWLYTICS_PASSWORD_HASH is invalid",
            ),
        ] {
            let mut fixture = Fixture::new();
            if existing_database {
                healthcheck(&fixture.boot());
                fixture.stop();
            }
            let error = fixture.startup_error(hash);
            assert!(error.contains("No stored owner credential"), "{error}");
            assert!(error.contains(message), "{error}");
            assert!(error.contains("owlytics hash-password"), "{error}");
            if let Some(hash) = hash.filter(|hash| !hash.is_empty()) {
                assert!(!error.contains(hash), "must not print the supplied hash");
            }
        }
    }
}

fn csrf(html: &str) -> String {
    html.split("name=\"csrf\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned()
}

fn cookie(response: &reqwest::Response) -> String {
    response.headers()[reqwest::header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn login(client: &reqwest::Client, address: &str, password: &str) -> reqwest::Response {
    let page = client
        .get(format!("http://{address}/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    let pre_cookie = cookie(&page);
    let token = csrf(&page.text().await.unwrap());
    client
        .post(format!("http://{address}/login"))
        .header("Origin", "http://localhost:3000")
        .header("Cookie", pre_cookie)
        .form(&[("csrf", token.as_str()), ("password", password)])
        .send()
        .await
        .unwrap()
}

async fn settings(client: &reqwest::Client, address: &str, session: &str) -> reqwest::Response {
    client
        .get(format!("http://{address}/settings"))
        .header("Cookie", session)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn environment_credentials_remain_active_until_changed_in_settings() {
    let mut fixture = Fixture::new();
    let client = client();
    let address = fixture.boot();
    let signed_in = login(&client, &address, common::PASSWORD).await;
    assert_eq!(signed_in.status(), 303);
    let initial_session = cookie(&signed_in);
    fixture.stop();

    // Environment-only installations still change passwords by changing the hash.
    let environment_password = "a replacement environment password";
    let environment_hash = owlytics::auth::hash_password(environment_password).unwrap();
    let address = fixture.boot_with_hash(Some(&environment_hash));
    assert_eq!(
        settings(&client, &address, &initial_session).await.status(),
        303
    );
    assert_eq!(
        login(&client, &address, common::PASSWORD).await.status(),
        401
    );
    let signed_in = login(&client, &address, environment_password).await;
    assert_eq!(signed_in.status(), 303);
    let session = cookie(&signed_in);
    let connection = rusqlite::Connection::open(fixture.database()).unwrap();
    assert!(
        owlytics::auth::stored_owner_hash(&connection)
            .unwrap()
            .is_none()
    );

    let page = settings(&client, &address, &session).await;
    assert_eq!(page.status(), 200);
    let token = csrf(&page.text().await.unwrap());
    let password = "a password saved through settings";
    let changed = client
        .post(format!("http://{address}/settings"))
        .header("Origin", "http://localhost:3000")
        .header("Cookie", &session)
        .form(&[
            ("csrf", token.as_str()),
            ("current_password", environment_password),
            ("new_password", password),
            ("confirm_password", password),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), 303);
    assert_eq!(settings(&client, &address, &session).await.status(), 303);
    let signed_in = login(&client, &address, password).await;
    assert_eq!(signed_in.status(), 303);
    let stored_session = cookie(&signed_in);
    fixture.stop();

    // The saved password and its sessions survive restarts regardless of the fallback.
    let unsupported = common::hash().replace("m=7168", "m=1048576");
    for hash in [
        Some(common::hash()),
        None,
        Some(""),
        Some("invalid-hash"),
        Some(&unsupported),
    ] {
        // Keep these rapid restarts independent of the persistent login throttle.
        connection
            .execute(
                "UPDATE login_limit SET window_start=?1",
                [owlytics::auth::now() - 61],
            )
            .unwrap();
        let address = fixture.boot_with_hash(hash);
        healthcheck(&address);
        assert_eq!(
            settings(&client, &address, &stored_session).await.status(),
            200
        );
        assert_eq!(
            login(&client, &address, common::PASSWORD).await.status(),
            401
        );
        assert_eq!(
            login(&client, &address, environment_password)
                .await
                .status(),
            401
        );
        assert_eq!(login(&client, &address, password).await.status(), 303);
        fixture.stop();
    }
}

#[tokio::test]
async fn cli_recovery_without_environment_hash_revokes_sessions_and_persists() {
    let mut fixture = Fixture::new();
    let client = client();
    let address = fixture.boot();
    let signed_in = login(&client, &address, common::PASSWORD).await;
    assert_eq!(signed_in.status(), 303);
    let mut session = cookie(&signed_in);
    let mut previous_password = common::PASSWORD;

    // Reject bad input without replacing credentials or revoking valid sessions.
    let rejected = fixture.reset_password("short\n", None);
    assert!(!rejected.status.success());
    assert_eq!(settings(&client, &address, &session).await.status(), 200);

    for (hash, password) in [
        (None, "a recovered password without environment"),
        (
            Some("invalid-hash"),
            "another recovered password with invalid environment",
        ),
    ] {
        let connection = rusqlite::Connection::open(fixture.database()).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO login_limit VALUES(1,?1,10)",
                [owlytics::auth::now()],
            )
            .unwrap();
        let output = fixture.reset_password(&format!("{password}\n"), hash);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "Password reset. All sessions signed out."
        );
        for table in ["owner_sessions", "login_limit"] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        assert_eq!(settings(&client, &address, &session).await.status(), 303);
        assert_eq!(
            login(&client, &address, previous_password).await.status(),
            401
        );
        let signed_in = login(&client, &address, password).await;
        assert_eq!(signed_in.status(), 303);
        session = cookie(&signed_in);
        previous_password = password;
    }
    fixture.stop();

    for hash in [None, Some("invalid-hash"), Some(common::hash())] {
        let address = fixture.boot_with_hash(hash);
        assert_eq!(settings(&client, &address, &session).await.status(), 200);
        assert_eq!(
            login(&client, &address, common::PASSWORD).await.status(),
            401
        );
        assert_eq!(
            login(&client, &address, previous_password).await.status(),
            303
        );
        fixture.stop();
    }
}

#[test]
fn password_cli_uses_stdin_and_rejects_short_passwords() {
    fn hash(input: &str) -> std::process::Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_owlytics"))
            .arg("hash-password")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    let output = hash("a long password from stdin\n");
    assert!(output.status.success());
    let value = String::from_utf8(output.stdout).unwrap();
    assert!(owlytics::auth::validate_hash(value.trim()));
    assert!(owlytics::auth::verify_password(
        "a long password from stdin",
        value.trim()
    ));
    let output = hash("short\n");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn startup_rejects_a_newer_database_schema() {
    let fixture = Fixture::new();
    let database = fixture.directory.join("future.sqlite");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 999")
        .unwrap();
    drop(connection);
    assert!(matches!(
        owlytics::db::Database::open(&database),
        Err(owlytics::db::Error::NewerSchema)
    ));
    let connection = rusqlite::Connection::open(database).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 999);
}

#[tokio::test]
async fn plausible_cli_previews_then_imports_and_rejects_duplicate_imports() {
    let fixture = Fixture::new();
    let path = fixture.directory.join("analytics.sqlite");
    let db = owlytics::db::Database::open(&path).unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let directory = fixture.directory.join("plausible export");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("imported_visitors_20200101_20200101.csv"),
        "date,visitors,pageviews\n2020-01-01,3,8\n",
    )
    .unwrap();
    fs::write(
        directory.join("imported_custom_props_20200101_20200101.csv"),
        "outside scope\n",
    )
    .unwrap();
    let run = |options: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_owlytics"))
            .env("OWLYTICS_DATABASE", &path)
            .args(["import-plausible", "example.com"])
            .arg(&directory)
            .args(options)
            .output()
            .unwrap()
    };
    let preview = run(&["--dry-run"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let output = String::from_utf8(preview.stdout).unwrap();
    assert!(output.contains("Would import 3 visitors and 8 pageviews"));
    assert!(output.contains("Dry run: no imported data saved."));
    assert!(output.contains("Outside v1 scope: custom_props"));
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        0
    );
    assert!(!run(&["--unknown"]).status.success());
    let imported = run(&[]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(
        String::from_utf8(imported.stdout)
            .unwrap()
            .contains("Imported 3 visitors and 8 pageviews")
    );
    let repeated = run(&[]);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("already been imported"));
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT SUM(pageviews) FROM import_daily", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        8
    );
}
