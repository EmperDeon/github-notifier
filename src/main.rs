use actix_web::{get, App, HttpServer, Responder};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::time::sleep;

use lazy_static::lazy_static;
use prometheus::register_int_gauge;
use prometheus::register_int_gauge_vec;
use prometheus::{self, Encoder, IntGauge, IntGaugeVec, TextEncoder};

use serde_derive::{Deserialize, Serialize};

lazy_static! {
  pub static ref VERSION: IntGaugeVec = register_int_gauge_vec!(
    "github_notifier_version",
    "Report current application version",
    &["version"]
  )
  .unwrap();
  pub static ref ELAPSED: IntGaugeVec = register_int_gauge_vec!(
    "github_notifier_elapsed",
    "Milliseconds of various places in project",
    &["type"]
  )
  .unwrap();
  pub static ref LAST_UPDATED: IntGauge =
    register_int_gauge!("github_notifier_last_updated", "Timestamp of last update").unwrap();
}

#[get("/metrics")]
async fn metrics() -> impl Responder {
  let mut buffer = Vec::new();
  let encoder = TextEncoder::new();

  let metric_families = prometheus::gather();

  // Upwraps here are safe
  encoder.encode(&metric_families, &mut buffer).unwrap();
  String::from_utf8(buffer.clone()).unwrap()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  pretty_env_logger::formatted_timed_builder()
    .filter(
      Some(&env!("CARGO_PKG_NAME").replace('-', "_")),
      log::LevelFilter::from_str(&env::var("RUST_LOG").unwrap_or_else(|_| String::from("info"))).unwrap(),
    )
    .init();
  log::info!("Booting up");

  // Read config
  let state_file = env::var("STATE_FILE").unwrap_or("/home/replace-me/.config/github-notifier.json".to_owned());
  let telegram_token = env::var("TELEGRAM_TOKEN")
    .expect("Please provide a telegram token for release notifications (TELEGRAM_TOKEN env). Format: bot_id:bot_token");
  let telegram_chat = env::var("TELEGRAM_CHAT").expect("Please provide a release notifications destination (TELEGRAM_CHAT env). Format: number, can be found with @myidbot Telegram bot");
  let period_secs: u64 = env::var("PERIOD_SECS").unwrap_or("3600".to_owned()).parse()?;
  let repos = env::var("REPOS").unwrap_or("".to_owned());
  let search = env::var("SEARCH").unwrap_or("".to_owned());
  let metrics_addr = env::var("METRICS_ADDR").unwrap_or("127.0.0.1".to_owned());
  let metrics_port: u16 = env::var("METRICS_PORT").unwrap_or("8080".to_owned()).parse()?;

  let search_r = regex::Regex::new(&search)?;

  VERSION
    .get_metric_with_label_values(&[env!("GITHUB_REF")])
    .unwrap()
    .set(1);

  let state = read_state(state_file.clone());
  write_state(state_file.clone(), &state);

  tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(period_secs));

    let repo_list: Vec<&str> = repos.split(",").collect();

    loop {
      interval.tick().await;
      let now = Instant::now();

      let mut state = read_state(state_file.clone());

      for repo in &repo_list {
        match fetch_latest_release(repo).await {
          Ok(release) => {
            log::info!("Found release: {:?}", release);
            let important = !search.is_empty() && search_r.is_match(&release.body);

            if !state.sent_releases.get(repo.to_owned()).unwrap_or(&0).eq(&release.id) {
              state.sent_releases.insert(repo.to_owned().to_owned(), release.id);

              send_notification(repo, release, important, &telegram_token, &telegram_chat).await;

              sleep(Duration::from_secs(2)).await;
            }
          }
          Err(err) => {
            log::error!("Check repo error: {:?}", err);
          }
        }
      }

      write_state(state_file.clone(), &state);

      log::debug!("Finished gathering releases");
      ELAPSED
        .get_metric_with_label_values(&["chain_list"])
        .unwrap()
        .set(now.elapsed().as_millis() as i64);
      LAST_UPDATED.set(chrono::Utc::now().timestamp_millis());
    }
  });

  log::info!(
    "Started server, looking for releases in Repos: {}",
    env::var("REPOS").unwrap_or("".to_owned())
  );
  let _ = HttpServer::new(|| App::new().service(metrics))
    .bind((metrics_addr, metrics_port))
    .unwrap()
    .run()
    .await;

  Ok(())
}

#[derive(Deserialize, Clone, Debug)]
struct Release {
  id: u32,
  html_url: String,
  name: String,
  prerelease: bool,
  draft: bool,
  body: String,
  published_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct State {
  sent_releases: HashMap<String, u32>,
}

fn read_state(file: String) -> State {
  let file = match File::open(file) {
    Ok(f) => f,
    Err(err) => {
      if err.kind() == std::io::ErrorKind::NotFound {
        return State::default();
      } else {
        panic!("Error on reading state: {:?}", err);
      }
    }
  };
  // Fail on errors
  serde_json::from_reader(file).unwrap()
}

fn write_state(file: String, state: &State) {
  let file = File::create(file).unwrap();
  serde_json::to_writer_pretty(file, state).unwrap();
}

async fn fetch_latest_release(repo: &str) -> anyhow::Result<Release> {
  let client = reqwest::Client::new();
  let response = client.get(format!("https://api.github.com/repos/{repo}/releases"))
      .header("Content-Type", "application/json")
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36 Edg/119.0.0.0").send().await?.text().await?;

  let mut releases: Vec<Release> = serde_json::from_str(&response)?;
  releases.sort_by(|a, b| a.id.cmp(&b.id));
  releases.retain(|e| !e.prerelease && !e.draft);

  log::debug!("Found {} releases", releases.len());
  Ok(
    releases
      .last()
      .ok_or(anyhow::anyhow!("No releases in repo {repo}"))?
      .clone(),
  )
}

async fn send_notification(
  repo: &str,
  release: Release,
  important: bool,
  telegram_token: &String,
  telegram_chat: &String,
) {
  let client = reqwest::Client::new();
  let text = format!(
                "{}New release <a href=\"{}\">{}</a> in <b>{}</b> at {}\n\n<pre><code class=\"language-markdown\">{:.3900}</code></pre>",
                if important { "🐅" } else { "🦦" },
                release.html_url,
                release.name,
                repo,
                release.published_at,
                release.body
              );
  let params = [
    ("chat_id", telegram_chat.clone()),
    ("text", text),
    ("parse_mode", "HTML".to_owned()),
    ("disable_notification", (!important).to_string()),
  ];

  if let Err(err) = client
    .post(format!(
      "https://api.telegram.org/bot{}/sendMessage",
      telegram_token.clone()
    ))
    .form(&params)
    .send()
    .await
  {
    log::error!("Failed to send notification: {:?}", err);
  }
}
