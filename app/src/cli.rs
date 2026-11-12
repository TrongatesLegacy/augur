//! `augur <command>`: the engine from the command line, sharing the app's data folder. The Claude skills use this.

use crate::settings::{self, AppSettings};
use augur_engine::{backfill, calls, db, history, model, predict, rusqlite, site};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

/// Engine errors and the command line's own both become text when printed.
pub type CliResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(name = "augur", version, about = "Calls PalDuels fights before the bell. Run without a command for the app.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Predict the duel that's up now: the pick, the five chat lines, then the case. Saves the call.
    Predict {
        /// skip the owner/pal head-to-heads and hidden-move lookups (about a second faster)
        #[arg(long)]
        quick: bool,
        /// print the whole prediction as JSON
        #[arg(long)]
        json: bool,
        /// predict from a saved /api/state.php response instead of the live one (doesn't save the call)
        #[arg(long)]
        state: Option<PathBuf>,
    },
    /// Fetch every species and past duels into the store (gently; pauses while a duel is up).
    Backfill {
        /// only the species pages
        #[arg(long)]
        species_only: bool,
    },
    /// Load calls from a calls file (JSON Lines), e.g. fixtures/calls.jsonl.
    Import { file: PathBuf },
    /// Write every settled call to a calls file.
    Export { file: PathBuf },
    /// The tally of real calls, calibration, the history test and the species leaning on unlisted moves.
    Analyse,
    /// Re-test model settings on the real calls: SETTING=v1,v2 ... (TYPE_BONUS, TYPE_RESIST, POWER_EXP,
    /// STATS_STEEPNESS, SPECIES_WEIGHT, PAL_H2H_WEIGHT). Changes nothing.
    Tune { grid: Vec<String> },
    /// Where the data lives and how much of it there is.
    Status,
}

pub fn run(cli: Cli) -> CliResult {
    let dir = settings::data_dir();
    let conn = db::open(&settings::db_path(&dir))?;
    let app = AppSettings::load(&dir);
    match cli.command {
        Command::Predict { quick, json, state: saved } => {
            let state: site::State = match &saved {
                Some(f) => serde_json::from_str(&std::fs::read_to_string(f)?)?,
                None => site::State::fetch()?,
            };
            if saved.is_none() {
                db::settle(&conn, &state.recent, |id| site::duel(id).ok())?;
            }
            match predict::predict(
                &state,
                &predict::Options { lookups: !quick, db_path: settings::db_path(&dir), settings: app.model.clone() },
            ) {
                None => println!("No duel up right now ({}).", if state.title.is_empty() { "idle" } else { &state.title }),
                Some(p) => {
                    if saved.is_none() {
                        db::add_call(&conn, &p.to_call())?;
                    }
                    if json {
                        println!("{}", serde_json::to_string_pretty(&p).unwrap());
                    } else {
                        println!("{}", predict::report(&p));
                    }
                }
            }
            print_tally(&conn, &app.model)
        }
        Command::Backfill { species_only } => {
            let stop = AtomicBool::new(false);
            let busy = || site::State::fetch().map(|s| s.busy()).unwrap_or(false);
            let progress = |p: &backfill::Progress| {
                eprintln!(
                    "{} {}/{}{}{}",
                    p.phase,
                    p.done,
                    p.total,
                    if p.paused { " (paused: a duel is up)" } else { "" },
                    p.last_error.as_ref().map(|e| format!(" ({e}, delay now {}ms)", p.delay_ms)).unwrap_or_default()
                )
            };
            let ctl = backfill::Control { stop: &stop, busy: &busy, progress: &progress };
            if species_only {
                backfill::species(&conn, &mut backfill::Pacer::new(std::time::Duration::from_millis(500)), &ctl, false)?;
            } else {
                backfill::run(&conn, &ctl)?;
            }
            Ok(())
        }
        Command::Import { file } => {
            let n = calls::import(&conn, &file)?;
            println!("{n} calls imported into {}", settings::db_path(&dir).display());
            Ok(())
        }
        Command::Export { file } => {
            let n = calls::export(&conn, &file)?;
            println!("{n} calls written to {}", file.display());
            Ok(())
        }
        Command::Analyse => analyse(&conn, &app.model),
        Command::Tune { grid } => tune(&conn, &app.model, &grid),
        Command::Status => {
            let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
            println!("Data folder: {}", dir.display());
            println!(
                "Calls: {} ({} settled)",
                count("SELECT COUNT(*) FROM predictions"),
                count("SELECT COUNT(*) FROM predictions WHERE status = 'settled'")
            );
            println!(
                "Duels: {} ({} with tapes, {} hits)",
                count("SELECT COUNT(*) FROM duels"),
                count("SELECT COUNT(DISTINCT duel_id) FROM hits"),
                count("SELECT COUNT(*) FROM hits")
            );
            println!("Species: {}", count("SELECT COUNT(*) FROM species"));
            Ok(())
        }
    }
}

fn print_tally(conn: &rusqlite::Connection, s: &model::Settings) -> CliResult {
    let calls = db::settled(conn)?;
    let t = history::tally(&calls, None);
    println!("\n  TALLY: {}/{} right", t.right, t.total);
    for c in calls.iter().rev().take(5).rev() {
        println!(
            "    {} {} vs {}: picked {} ({:.0}%), {} won",
            if c.right() { "OK" } else { "XX" },
            c.home_pal,
            c.away_pal,
            c.pick,
            c.prob * 100.0,
            c.winner.as_deref().unwrap_or("?")
        );
    }
    let (_, right, n) = history::evaluate(&calls, s);
    println!(
        "\n  WITH TODAY'S LOGIC: {right}/{n} right (was {}/{n} on these fights)",
        calls.iter().filter(|c| c.fighters.is_some() && c.species_rec.is_some() && c.right()).count()
    );
    Ok(())
}

fn analyse(conn: &rusqlite::Connection, s: &model::Settings) -> CliResult {
    let calls = db::settled(conn)?;
    let t = history::tally(&calls, None);
    println!("REAL CALLS: {}/{} right, best run {}, average confidence {:.0}%", t.right, t.total, t.best_run, t.avg_confidence * 100.0);
    for (label, won, n, promised) in &t.calibration {
        println!("  {label:8} won {won}/{n}, promised {:.0}%", promised * 100.0);
    }
    if let Some((moves, fights, pa, edge)) = history::calibration(conn, &calls, s)? {
        println!("\nPER-HIT FIT ({moves} moves from {fights} fights): damage ~ power^{pa:.2}, a type edge is really ~{edge:.2}x (model uses {}x)", s.type_bonus);
    }
    let b = history::backtest(conn, s)?;
    println!(
        "\nHISTORY TEST on {} past duels (base stats, average roll): {:.1}% right, log loss {:.3}",
        b.duels,
        b.accuracy * 100.0,
        b.log_loss
    );
    println!("\nUNLISTED MOVES (species doing most damage with moves the site doesn't list):");
    for u in history::unlisted_leaders(conn, 300.0, 12)? {
        let m: Vec<String> = u.moves.iter().map(|(n, d)| format!("{n} {d:.0}%")).collect();
        println!("  {:20} {:>3.0}%  {}", u.pal, u.share * 100.0, m.join(", "));
    }
    Ok(())
}

fn tune(conn: &rusqlite::Connection, base: &model::Settings, grid: &[String]) -> CliResult {
    let calls = db::settled(conn)?;
    let parsed: Vec<(String, Vec<f64>)> = if grid.is_empty() {
        vec![
            ("TYPE_BONUS".into(), vec![1.25, 1.5, 2.0]),
            ("STATS_STEEPNESS".into(), vec![2.0, 3.0, 4.0]),
            ("SPECIES_WEIGHT".into(), vec![0.7, 1.2]),
        ]
    } else {
        grid.iter()
            .map(|g| {
                let (k, v) = g.split_once('=').ok_or(format!("expected SETTING=v1,v2, got {g}"))?;
                Ok((
                    k.to_uppercase(),
                    v.split(',').map(|x| x.trim().parse::<f64>().map_err(|e| format!("{x}: {e}"))).collect::<Result<_, _>>()?,
                ))
            })
            .collect::<Result<_, String>>()?
    };
    let grid: Vec<(&str, Vec<f64>)> = parsed.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let (loss, right, n) = history::evaluate(&calls, base);
    println!("{n} calls. Current settings: log loss {loss:.3}, {right}/{n} right");
    for (loss, right, s) in history::tune(&calls, base, &grid).into_iter().take(8) {
        println!("  log loss {loss:.3}  {right}/{n} right  {}", serde_json::to_string(&s).unwrap());
    }
    println!("These settings were scored on the same calls they were fitted to, so small gains may be chance.");
    Ok(())
}
