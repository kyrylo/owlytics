use owlytics::{config::Config, db::Database};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments == ["hash-password"] || arguments == ["reset-password"] {
        let hash = owlytics::auth::hash_password(&read_password()?)?;
        if arguments == ["hash-password"] {
            println!("{hash}");
        } else {
            let path = std::env::var("OWLYTICS_DATABASE")
                .unwrap_or_else(|_| "data/owlytics.sqlite".into());
            if !std::path::Path::new(&path).is_file() {
                return Err("Set OWLYTICS_DATABASE to an existing database".into());
            }
            let db = Database::open(path)?;
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    db.call(move |c| owlytics::auth::replace_owner_password(c, &hash))
                        .await
                })?;
            println!("Password reset. All sessions signed out.");
        }
        return Ok(());
    }
    if !arguments.is_empty() {
        return command(&arguments);
    }
    let config = Config::from_env()?;
    if let Some(parent) = config
        .database
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
    }
    let db = Database::open(&config.database)?;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(async {
            let maintenance = db.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    let _ = maintenance
                        .call(|c| owlytics::analytics::prune_salts(c, owlytics::auth::now()))
                        .await;
                }
            });
            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            println!("Owlytics listening on http://{}", listener.local_addr()?);
            axum::serve(
                listener,
                owlytics::router(db, config)
                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown())
            .await
        })?;
    Ok(())
}

fn read_password() -> Result<String, Box<dyn std::error::Error>> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return Err("Read the password through stdin from a hidden prompt; do not pass it as a command-line argument".into());
    }
    let mut password = String::new();
    std::io::stdin().take(1027).read_to_string(&mut password)?;
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    Ok(password)
}

fn command(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::path::Path;
    let database =
        std::env::var("OWLYTICS_DATABASE").unwrap_or_else(|_| "data/owlytics.sqlite".into());
    match arguments {
        [command, domain, directory, options @ ..] if command == "import-plausible" => {
            let mut dry_run = false;
            let mut allowed_hostnames = Vec::new();
            let mut options = options.iter();
            while let Some(option) = options.next() {
                match option.as_str() {
                    "--dry-run" if !dry_run => dry_run = true,
                    "--allow-hostname" => allowed_hostnames.push(options.next().ok_or("--allow-hostname requires a hostname label")?.clone()),
                    _ => return Err("Usage: owlytics import-plausible DOMAIN DIRECTORY [--dry-run] [--allow-hostname HOST]...".into()),
                }
            }
            if !Path::new(&database).is_file() { return Err("Add the website in Owlytics before importing".into()); }
            let domain = owlytics::analytics::normalize_domain(domain).ok_or("Enter a valid website domain")?;
            let db = Database::open(&database)?;
            let directory = std::path::PathBuf::from(directory);
            tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
                let site = owlytics::analytics::sites(&db).await?.into_iter().find(|site| site.domain == domain).ok_or("Add the website in Owlytics before importing")?;
                let today = site.today();
                let summary = db.call(move |c| Ok(owlytics::migration::import_with_hostnames(
                    c, &domain, &directory, today, &allowed_hostnames, dry_run,
                ))).await??;
                println!("{} {} visitors and {} pageviews, {} through {}", if dry_run { "Would import" } else { "Imported" }, summary.visitors, summary.pageviews, summary.from, summary.to);
                if dry_run { println!("Dry run: no imported data saved."); }
                if summary.skipped_rows > 0 { println!("Skipped {} rows on or after the collection cutover", summary.skipped_rows); }
                if !summary.ignored.is_empty() { println!("Outside v1 scope: {}", summary.ignored.join(", ")); }
                if !summary.missing.is_empty() { println!("Missing breakdowns: {}", summary.missing.join(", ")); }
                if summary.session_date_differences { println!("Some breakdown totals differ from the chart because Plausible groups them by session date."); }
                Ok(())
            })
        }
        [command, source] if command == "restore" => {
            owlytics::storage::restore(Path::new(source), Path::new(&database))?;
            println!("Restored to {database}");
            Ok(())
        }
        [command, file] if command == "import-geo" || command == "backup" => {
            if !Path::new(&database).is_file() { return Err("Start Owlytics once to create its database, or set OWLYTICS_DATABASE to an existing database".into()); }
            let db = Database::open(&database)?;
            let file = file.clone();
            tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
                if command == "import-geo" {
                    let input = std::fs::File::open(file)?;
                    let count = db.call(move |c| Ok(owlytics::storage::import_geo(c, input))).await??;
                    println!("Imported {count} country ranges");
                } else {
                    db.read(move |c| Ok(owlytics::storage::backup(c, Path::new(&file)))).await??;
                    println!("Backup saved");
                }
                Ok(())
            })
        }
        _ => Err("Usage: owlytics [hash-password | reset-password | import-plausible DOMAIN DIRECTORY [--dry-run] [--allow-hostname HOST]... | import-geo FILE.csv | backup NEW_FILE.sqlite | restore BACKUP.sqlite]".into()),
    }
}

async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = interrupt => {}, _ = terminate => {} }
}
