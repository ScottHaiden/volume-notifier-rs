use std::process::{
    Command,
    Stdio,
};
use std::fs::File;
use std::io::{
    Read,
    SeekFrom,
    Seek,
    Write,
};

use regex::Regex;
use clap::Parser;

fn default_path() -> String {
    let uid: libc::uid_t = unsafe { libc::getuid() };
    format!("/run/user/{}/volume.id", uid)
}

/// Simple program to change the volume and send a notification.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Path to database.
    #[arg(short = 'p', long, default_value = default_path())]
    db_path: std::path::PathBuf,

    /// Interval by which to increase and decrease the volume.
    #[arg(short = 'i', long, default_value = "512")]
    interval: i32,

    /// Sink on which to perform the action.
    #[arg(short = 's', long, default_value = "@DEFAULT_SINK@")]
    sink: String,

    /// Task
    #[arg(default_value = "noop")]
    task: String,
}

impl Args {
    fn get_command_or_die(&self) -> Vec<String> {
        match self.task.as_str() {
            "up" => vec![
                "pactl".into(),
                "set-sink-volume".into(),
                self.sink.clone(),
                format!("+{}", self.interval),
            ],
            "down" => vec![
                "pactl".into(),
                "set-sink-volume".into(),
                self.sink.clone(),
                format!("-{}", self.interval),
            ],
            "mute" => vec![
                "pactl".into(),
                "set-sink-mute".into(),
                self.sink.clone(),
                "toggle".into(),
            ],
            "noop" => vec!["true".into()],
            _ => {
                eprintln!("Unknown task {}", self.task);
                std::process::exit(1);
            },
        }
    }
}

fn read_db(db: &mut File) -> std::io::Result<Option<i32>> {
    let _ = db.seek(SeekFrom::Start(0));

    let mut contents = String::new();
    db.read_to_string(&mut contents)?;

    let trimmed = contents.trim();
    if trimmed.len() == 0 { return Ok(None); }

    Ok(Some(trimmed.parse::<>().expect("Failed to parse DB")))
}

fn write_db(db: &mut File, contents: i32) -> std::io::Result<()> {
    db.set_len(0)?;
    write!(db, "{}", contents)?;
    Ok(())
}

fn run_or_die(cmd: &[String]) -> String {
    let stdout: Vec<u8> = Command::new(cmd[0].clone())
        .args(&cmd[1..])
        .stderr(Stdio::inherit())
        .output()
        .expect("Failed to execute command")
        .stdout;

    String::from_utf8(stdout).expect("Failed to decode output").trim().into()
}

fn parse_volume(vol: &str) -> (u32, Vec<&str>) {
    let re = Regex::new(r"\S+: [0-9]+ / \s*([0-9]+)% / -?[0-9.]+ dB")
        .expect("RE failed to compile");

    let mut total = 0u32;
    let mut ret = Vec::<&str>::new();

    for (full, [pct]) in re.captures_iter(vol).map(|c| c.extract()) {
        ret.push(full);
        total += pct.parse::<u32>().unwrap();
    }

    (total / ret.len() as u32, ret)
}

fn get_icon(mutestr: &str, percent: u32) -> &'static str {
    if mutestr == "Mute: yes" { return "audio-volume-muted"; }

    match percent {
        0 => "audio-volume-muted",
        1..33 => "audio-volume-low",
        33..66 => "audio-volume-medium",
        _ => "audio-volume-high",
    }
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let _ = run_or_die(&args.get_command_or_die());

    let mute = run_or_die(&["pactl".into(), "get-sink-mute".into(), args.sink.clone()]);
    let volume = run_or_die(&["pactl".into(), "get-sink-volume".into(), args.sink.clone()]);
    let (vol_pct, channels) = parse_volume(&volume);

    let mut db = File::options()
        .read(true)
        .write(true)
        .create(true)
        .append(true)
        .open(args.db_path)?;
    db.lock_shared()?;

    // Read the database under a shared lock.
    let old_id: Option<i32> = if let Some(id) = read_db(&mut db)? {
        Some(id)
    } else {
        db.lock()?;
        read_db(&mut db)?
    };

    let channels = channels.into_iter()
        .map(|c| format!("- {}", c))
        .collect::<Vec<String>>()
        .join("\n");

    let mut notif_cmd = vec![
        "notify-send".into(),
        "Volume".into(),
        format!("{}\n{}", mute, channels),
        "-p".into(),
        "-i".into(), get_icon(&mute, vol_pct).into(),
    ];
    if let Some(id) = old_id { notif_cmd.extend(["-r".into(), format!("{}", id)]); }

    let new_id = run_or_die(&notif_cmd).parse::<i32>().expect("Failed to parse new ID");

    if let None = old_id { write_db(&mut db, new_id)?; }

    Ok(())
}
