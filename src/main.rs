use cal3::{CalendarHub, Error};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use google_calendar3 as cal3;
use hyper;
use hyper_rustls;
use oauth2::{
    ApplicationSecret, Authenticator, DefaultAuthenticatorDelegate, FlowType, Token, TokenStorage,
};
use serde_json as json;

use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use structopt::StructOpt;
use yup_oauth2 as oauth2;

use humantime::format_duration;

#[derive(StructOpt)]
struct Cli {
    // only time until next
    #[structopt(short = "t", long = "time")]
    time: bool,
    // get hangouts join link
    #[structopt(short = "j", long = "join")]
    hangouts: bool,
}

struct JsonTokenStorage {
    pub program_name: &'static str,
    pub db_dir: String,
}

impl JsonTokenStorage {
    fn path(&self, scope_hash: u64) -> PathBuf {
        Path::new(&self.db_dir).join(&format!("{}-token-{}.json", self.program_name, scope_hash))
    }
}

impl TokenStorage for JsonTokenStorage {
    type Error = TokenStorageError;

    // NOTE: logging might be interesting, currently we swallow all errors
    fn set(
        &mut self,
        scope_hash: u64,
        _: &Vec<&str>,
        token: Option<Token>,
    ) -> Result<(), TokenStorageError> {
        match token {
            None => match fs::remove_file(self.path(scope_hash)) {
                Err(err) => match err.kind() {
                    io::ErrorKind::NotFound => Ok(()),
                    _ => Err(TokenStorageError::Io(err)),
                },
                Ok(_) => Ok(()),
            },
            Some(token) => {
                match fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(&self.path(scope_hash))
                {
                    Ok(mut f) => match json::to_writer_pretty(&mut f, &token) {
                        Ok(_) => Ok(()),
                        Err(serde_err) => Err(TokenStorageError::Json(serde_err)),
                    },
                    Err(io_err) => Err(TokenStorageError::Io(io_err)),
                }
            }
        }
    }

    fn get(&self, scope_hash: u64, _: &Vec<&str>) -> Result<Option<Token>, TokenStorageError> {
        match fs::File::open(&self.path(scope_hash)) {
            Ok(f) => match json::de::from_reader(f) {
                Ok(token) => Ok(Some(token)),
                Err(err) => Err(TokenStorageError::Json(err)),
            },
            Err(io_err) => match io_err.kind() {
                io::ErrorKind::NotFound => Ok(None),
                _ => Err(TokenStorageError::Io(io_err)),
            },
        }
    }
}

#[derive(Debug)]
pub enum TokenStorageError {
    Json(json::Error),
    Io(io::Error),
}

impl fmt::Display for TokenStorageError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            TokenStorageError::Json(ref err) => writeln!(f, "Could not serialize secrets: {}", err),
            TokenStorageError::Io(ref err) => writeln!(f, "Failed to write secret token: {}", err),
        }
    }
}

impl StdError for TokenStorageError {
    fn description(&self) -> &str {
        "Failure when getting or setting the token storage"
    }
}

fn main() -> Result<(), Box<dyn StdError>> {
    let config_dir = ProjectDirs::from("com", "xortive", "meet").unwrap();

    fs::create_dir_all(config_dir.config_dir())?;

    let config_dir = config_dir.config_dir().to_str().unwrap();
    // Get an ApplicationSecret instance by some means. It contains the `client_id` and
    // `client_secret`, among other things.
    let secret: ApplicationSecret = oauth2::parse_application_secret(&r#"{"installed":{"client_id":"98200360730-j7ejmtbuka46jlusbd1tjupod92um2lc.apps.googleusercontent.com","project_id":"xortive-meet","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token","auth_provider_x509_cert_url":"https://www.googleapis.com/oauth2/v1/certs","client_secret":"5BygkFeSMC4nyb8U_TA31QRl","redirect_uris":["urn:ietf:wg:oauth:2.0:oob","http://localhost"]}}"#.to_string())?;
    // Instantiate the authenticator. It will choose a suitable authentication flow for you,
    // unless you replace  `None` with the desired Flow.
    // Provide your own `AuthenticatorDelegate` to adjust the way it operates and get feedback about
    // what's going on. You probably want to bring in your own `TokenStorage` to persist tokens and
    // retrieve them from storage.
    let auth = Authenticator::new(
        &secret,
        DefaultAuthenticatorDelegate,
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        JsonTokenStorage {
            program_name: "meet-cli",
            db_dir: config_dir.to_owned(),
        },
        Some(FlowType::InstalledRedirect(2383)),
    );
    let hub = CalendarHub::new(
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        auth,
    );
    // As the method needs a request, you would usually fill it with the desired information
    // into the respective structure. Some of the parts shown here might not be applicable !
    // Values shown here are possibly random and not representative !
    //let mut req = Channel::default();

    // You can configure optional parameters by calling the respective setters at will, and
    // execute the final call using `doit()`.
    // Values shown here are possibly random and not representative !
    let now = Utc::now();

    let result = hub
        .events()
        .list("primary")
        .time_min(now.to_rfc3339().as_ref())
        .single_events(true)
        .max_attendees(25)
        .order_by("startTime")
        .doit();

    let data = match result {
        Err(e) => match e {
            // The Error enum provides details about what exactly happened.
            // You can also just use its `Debug`, `Display` or `Error` traits
            Error::HttpError(_)
            | Error::MissingAPIKey
            | Error::MissingToken(_)
            | Error::Cancelled
            | Error::UploadSizeLimitExceeded(_, _)
            | Error::Failure(_)
            | Error::BadRequest(_)
            | Error::FieldClash(_)
            | Error::JsonDecodeError(_, _) => panic!("{:?}", e),
        },
        Ok((_, data)) => data,
    };

    let meetings = data
        .items
        .and_then(|events| {
            Some(events
                .into_iter()
                .filter(|event| event.summary.is_some() && event.start.is_some())
                .take(1)
                .peekable())
        });

    let mut meetings = meetings.expect("reading meetings from API response");

    if meetings.peek().is_none() {
        println!("Congrats! Keep working, you have no upcoming meetings");
    }

    for meeting in meetings {
        let summary = meeting.summary.unwrap();
        let start = meeting.start.unwrap();
        let start =
            DateTime::parse_from_rfc3339(start.date_time.unwrap().as_ref()).unwrap();

        let now = Utc::now();
        let time_until = start.signed_duration_since(now);
        let already_started = time_until < chrono::Duration::zero();
        let time_until = Duration::from_secs(time_until.num_seconds().abs() as u64);

        let location = meeting.location.map_or("".to_string(), move |l| format!("in location {}", l));

        println!("Next Meeting Details:");
        println!("{}", summary);
        if already_started {
            println!("Already started {} ago {}", format_duration(time_until), location);
        } else {
            println!("Starts in {} {}", format_duration(time_until), location);
        }
    }

    Ok(())
}
