//! Orchestra: a personal notes and databases app, on your own computer,
//! with no account and no subscription. AI comes through MCP; collaboration
//! through a connector you choose (see collab.rs).

// No console window behind the app on Windows (the MCP server still talks through the
// pipes its AI gives it).
#![cfg_attr(windows, windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod app;
mod auto;
mod awake;
mod clip;
mod collab;
mod docs;
mod live;
mod git;
mod mcp;
mod model;
mod opener;
mod project;
mod richtext;
mod screen;
mod secret;
mod secure;
mod sessions;
mod setup;
mod sync;
mod terminal;
mod shot;
mod store;
mod update;
mod usage;

const HELP: &str = "Orchestra — pages, blocs et bases de données, sur votre ordinateur.

Utilisation :
  orchestra ouvrir          ouvre la fenêtre (onglets de la dernière fois)
  orchestra orchestra://page/<id>  ouvre la fenêtre sur cette page
  orchestra mcp             serveur MCP (stdio) pour brancher une IA
  orchestra statusline      ligne d'état de Claude Code (relève l'utilisation de l'abonnement)
  orchestra export <dossier>  exporte toutes les pages en Markdown
  orchestra import <fichier.md>  importe un fichier Markdown comme page
  orchestra sync            synchronise une fois avec le branchement configuré
  orchestra serveur [port] [code]  point de rencontre en direct, sans fenêtre (port 7878 par défaut)
  orchestra aide            cette aide
  orchestra version         la version

Lancée sans argument hors d'un terminal (Finder, raccourci), elle ouvre la fenêtre.
Données : $ORCHESTRA_DIR, sinon le dossier de données de l'utilisateur.";

/// The original's console behaviour (capture/runs): run from a terminal with no
/// arguments, `--help` or `--version`, it prints nothing and exits with code 1
/// (the launcher at entry 0x100000460 hands everything to Electron, which has no
/// such options). Kept as-is; help and version live under `aide` / `version`.
const ORIGINAL_CONSOLE_EXIT: i32 = 1;

fn in_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        // From a terminal, like the original: nothing printed, exit 1.
        // From Finder, a shortcut or Craft Anything's Play: the window opens.
        None if in_terminal() => ORIGINAL_CONSOLE_EXIT,
        None => run_window(None),
        Some("--help") | Some("--version") => ORIGINAL_CONSOLE_EXIT,
        Some("ouvrir") | Some("open") => run_window(None),
        Some("-h") | Some("aide") | Some("help") => {
            println!("{HELP}");
            0
        }
        Some("-V") | Some("version") => {
            println!("orchestra {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("mcp") => mcp::run(),
        Some("statusline") => {
            usage::statusline_main();
            0
        }
        Some("export") => match args.get(1) {
            Some(dir) => {
                let ws = store::load();
                match store::export_all(&ws, std::path::Path::new(dir)) {
                    Ok(n) => {
                        println!("{n} page(s) exportée(s) dans {dir}");
                        0
                    }
                    Err(e) => {
                        eprintln!("orchestra: export impossible : {e}");
                        1
                    }
                }
            }
            None => usage("export a besoin d'un dossier"),
        },
        Some("import") => match args.get(1) {
            Some(file) => match std::fs::read_to_string(file) {
                Ok(md) => {
                    let mut ws = store::load();
                    let stem = std::path::Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or("Import").to_string();
                    let page = store::page_from_markdown(&md, &stem);
                    let title = page.display_title();
                    ws.pages.push(page);
                    match store::save(&ws) {
                        Ok(()) => {
                            println!("page « {title} » importée");
                            0
                        }
                        Err(e) => {
                            eprintln!("orchestra: enregistrement impossible : {e}");
                            1
                        }
                    }
                }
                Err(e) => {
                    eprintln!("orchestra: lecture impossible de {file} : {e}");
                    1
                }
            },
            None => usage("import a besoin d'un fichier"),
        },
        Some("serveur") | Some("server") => {
            let port = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(live::DEFAULT_PORT);
            let code = args.get(2).cloned().unwrap_or_else(|| model::new_id()[..8].to_string());
            run_server(port, &code)
        }
        Some("sync") => {
            let mut ws = store::load();
            match collab::sync(&mut ws) {
                Ok(report) => match store::save(&ws) {
                    Ok(()) => {
                        println!("{report}");
                        0
                    }
                    Err(e) => {
                        eprintln!("orchestra: enregistrement impossible : {e}");
                        1
                    }
                },
                Err(e) => {
                    eprintln!("orchestra: synchronisation impossible : {e}");
                    1
                }
            }
        }
        // macOS may pass a process serial number when the app is opened from Finder.
        Some(a) if a.starts_with("-psn_") => run_window(None),
        // A link copied with Cmd/Ctrl+L.
        Some(a) if a.starts_with("orchestra://page/") => run_window(Some(a["orchestra://page/".len()..].trim_end_matches('/').to_string())),
        Some(other) => usage(&format!("option inconnue : {other}")),
    };
    std::process::exit(code);
}

/// Headless meeting point for real-time collaboration: relays every change between the people
/// connected, keeps its own merged copy (in the data folder) and gives it to whoever joins, so
/// work isn't lost when the others are offline. Runs until stopped (Ctrl+C).
fn run_server(port: u16, code: &str) -> i32 {
    use live::LiveMsg;
    let mut ws = store::load();
    let server = match live::host(port, code, std::sync::Arc::new(|| {})) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("orchestra: impossible d'écouter sur le port {port} : {e}");
            return 1;
        }
    };
    println!("Serveur en direct sur {} (port {}), code « {code} »", server.status().address, server.port().unwrap_or(port));
    println!("Données : {}", store::workspace_path().display());
    loop {
        for msg in server.drain() {
            match msg {
                LiveMsg::Hello { name, .. } => {
                    println!("+ {name}");
                    server.send(&LiveMsg::Pages { pages: ws.pages.clone(), deleted: ws.deleted.clone() });
                    server.send(&LiveMsg::Chat { msgs: ws.chat.values().cloned().collect() });
                }
                LiveMsg::Bye { name } => println!("- {name}"),
                LiveMsg::Pages { pages, deleted } => {
                    let other = model::Workspace { pages, deleted, ..Default::default() };
                    if collab::merge(&mut ws, &other) > 0 || !other.deleted.is_empty() {
                        if let Err(e) = store::save(&ws) {
                            eprintln!("orchestra: enregistrement impossible : {e}");
                        }
                    }
                }
                LiveMsg::Chat { msgs } => {
                    let mut new = false;
                    for m in msgs {
                        new |= ws.chat.insert(m.id.clone(), m).is_none();
                    }
                    if new {
                        if let Err(e) = store::save(&ws) {
                            eprintln!("orchestra: enregistrement impossible : {e}");
                        }
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn usage(msg: &str) -> i32 {
    eprintln!("orchestra: {msg}\n\n{HELP}");
    2
}

/// 1320×860 as in the original; ORCHESTRA_WINDOW=WxH overrides it (developer aid).
fn window_size() -> [f32; 2] {
    use model::original::*;
    std::env::var("ORCHESTRA_WINDOW")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some([w.trim().parse().ok()?, h.trim().parse().ok()?])))
        .unwrap_or([DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT])
}

/// The app's icon (window, Dock, taskbar), from assets/icon-256.png.
fn app_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default()
}

fn run_window(open: Option<String>) -> i32 {
    use model::original::*;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Orchestra")
            .with_icon(std::sync::Arc::new(app_icon()))
            .with_inner_size(window_size())
            .with_min_inner_size([MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT])
            // Like the original (titleBarStyle "hiddenInset"): content runs under a transparent
            // title bar, with the window buttons over the sidebar. No effect off macOS.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    match eframe::run_native("Orchestra", options, Box::new(move |cc| Ok(Box::new(app::NotesApp::new(cc, open))))) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("orchestra: impossible d'ouvrir la fenêtre : {e}");
            1
        }
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn icon_decodes() {
        let i = super::app_icon();
        assert_eq!((i.width, i.height), (256, 256));
    }
}
