//use std::str::FromStr;

use dotenvy;
use tokio_postgres::NoTls;
//use refinery::config::Config;

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A .env file is optional (e.g. in containerized runs env vars come from
    // the orchestrator), so ignore a missing file instead of failing.
    dotenvy::dotenv().ok();
    let host = std::env::var("DB_HOST").expect("NO ENV HOST SET");
    let port = std::env::var("DB_PORT").unwrap_or_else(|_| "5432".to_string());
    let username = std::env::var("DB_USERNAME").or_else(|_| std::env::var("DEFAULT_USER"))
        .expect("NO ENV USERNAME SET (DB_USERNAME or DEFAULT_USER)");
    let password = std::env::var("DB_PASSWORD").or_else(|_| std::env::var("DEFAULT_PASSWORD"))
        .expect("NO ENV PASSWORD SET (DB_PASSWORD or DEFAULT_PASSWORD)");
    let dbname = std::env::var("DB_NAME").unwrap_or_else(|_| username.clone());
    let (mut client, connection) = tokio_postgres::connect(
        &format!(
            "host={host} port={port} user={username} password={password} dbname={dbname}"
        ),
        NoTls,
    )
    .await?;

    // The connection object performs the actual communication with the database,
    // so spawn it off to run on its own.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    // there is another way to set up a connection, in where we directly use
    // refinery tools since Config implements AsyncMigrate
    //let db_connection = format!("postgres://{username}:{password}@{host}:5432");
    //let mut config = Config::from_str(&db_connection)?;

    embedded::migrations::runner()
        .run_async(&mut client)
        .await?;

    println!("Migrations ran successfully");

    Ok(())
}
