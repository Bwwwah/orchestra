//! Orchestra: a personal notes and databases app, on your own computer,
//! with no account and no subscription. AI comes through MCP; collaboration
//! through a connector you choose (see collab.rs).

// No console window behind the app on Windows (the MCP server still talks through the
// pipes its AI gives it).
#![cfg_attr(windows, windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod app;
mod audit;
mod auto;
mod awake;
mod catalog;
mod clip;
mod collab;
mod crdt;
mod docs;
mod drive;
mod git;
mod identity;
mod live;
mod mcp;
mod model;
mod opener;
mod proc;
mod project;
mod review;
mod richtext;
mod rtk_stats;
mod screen;
mod secret;
mod secure;
mod pricing;
mod sessions;
mod setup;
mod shot;
mod space;
mod store;
mod sync;
mod team;
mod terminal;
mod triage;
mod update;
mod usage;

/// Lock a value shared with a background thread even when that thread panicked while
/// holding it: the value is taken as it is, rather than bringing the whole app down.
pub(crate) trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for std::sync::Mutex<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

const HELP: &str = "Orchestra — pages, blocs et bases de données, sur votre ordinateur.

Utilisation :
  orchestra ouvrir          ouvre la fenêtre (onglets de la dernière fois)
  orchestra orchestra://page/<id>  ouvre la fenêtre sur cette page
  orchestra mcp             serveur MCP (stdio) pour brancher une IA
  orchestra statusline      ligne d'état de Claude Code (relève l'utilisation de l'abonnement)
  orchestra nettoyer        avant de désinstaller : remet la ligne d'état de Claude Code et débranche les IA
  orchestra export <dossier>  exporte toutes les pages en Markdown
  orchestra import <fichier.md>  importe un fichier Markdown comme page
  orchestra serveur [port] [code]  point de rencontre en direct, sans fenêtre (port 7878 par défaut)
  orchestra equipe [port]   serveur d'équipe : un espace partagé par ses membres (port 7879)
  orchestra equipe inviter [ws://adresse:port] [lecteur]  une invitation à usage unique
  orchestra equipe membres  les membres, leur rôle, leur clé
  orchestra equipe retirer <nom|clé>  retire un membre (coupé dans les secondes qui suivent)
  orchestra equipe reintegrer <nom|clé>  lui rend l'accès
  orchestra equipe role <nom|clé> <admin|membre|lecteur>
  orchestra equipe ia-espace ajouter <nom> <claude-code|codex> <url> [modèle] [chat]
  orchestra equipe ia-espace cle <nom> <membre|tous> <clé>
  orchestra equipe ia-espace retirer <nom> | liste
  orchestra equipe alleger [jours]  allège l'historique des blocs inchangés depuis 7 jours
  orchestra equipe nom <nom>  le nom de l'espace
  orchestra equipe ia <libres|espace|aucune> [ia…]  les IA autorisées dans l'espace
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
        // The responder, started by the MCP server for a question an AI asked (see triage.rs).
        Some("trier") => triage::run(args.get(1).map(String::as_str)),
        Some("statusline") => {
            usage::statusline_main();
            0
        }
        // Before uninstalling (the Windows uninstaller runs it): nothing left behind that
        // would try to start Orchestra again.
        Some("nettoyer") | Some("cleanup") => {
            for line in setup::clean_up_for_uninstall() {
                println!("{line}");
            }
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
                    // From the read to the write: an AI's write through MCP isn't overwritten.
                    let _lock = store::lock_workspace();
                    let mut ws = store::load();
                    let stem = std::path::Path::new(file)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Import")
                        .to_string();
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
            let port = args
                .get(1)
                .and_then(|p| p.parse().ok())
                .unwrap_or(live::DEFAULT_PORT);
            let code = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| model::new_id()[..8].to_string());
            run_server(port, &code)
        }
        Some("equipe") | Some("team") => run_team(&args[1..]),
        Some(a) if a.starts_with("-psn_") => run_window(None),
        // A link copied with Cmd/Ctrl+L.
        Some(a) if a.starts_with("orchestra://page/") => run_window(Some(
            a["orchestra://page/".len()..]
                .trim_end_matches('/')
                .to_string(),
        )),
        Some(other) => usage(&format!("option inconnue : {other}")),
    };
    std::process::exit(code);
}

/// Headless meeting point for real-time collaboration: relays every change between the people
/// connected, keeps its own merged copy (in the data folder) and gives it to whoever joins, so
/// work isn't lost when the others are offline. Runs until stopped (Ctrl+C).
fn run_server(port: u16, code: &str) -> i32 {
    use live::LiveMsg;
    let server = match live::host(port, code, std::sync::Arc::new(|| {})) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("orchestra: impossible d'écouter sur le port {port} : {e}");
            return 1;
        }
    };
    println!(
        "Serveur en direct sur {} (port {}), code « {code} »",
        server.status().address,
        server.port().unwrap_or(port)
    );
    println!("Données : {}", store::workspace_path().display());
    loop {
        for msg in server.drain() {
            match msg {
                LiveMsg::Hello { name, .. } => {
                    println!("+ {name}");
                    for m in welcome(&store::load()) {
                        server.send(&m);
                    }
                }
                LiveMsg::Bye { name } => println!("- {name}"),
                m @ (LiveMsg::Pages { .. } | LiveMsg::Chat { .. }) => {
                    // From the read to the write, like the MCP server: what it or the window
                    // wrote meanwhile is merged into, never overwritten by a copy kept in memory.
                    let _lock = store::lock_workspace();
                    let mut ws = store::load();
                    if absorb(&mut ws, m)
                        && let Err(e) = store::save(&ws)
                    {
                        eprintln!("orchestra: enregistrement impossible : {e}");
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// What the server gives to whoever joins: only the shared projects, their pages and
/// discussions (`shared_copy`). The data folder it runs on may be someone's own, with
/// private pages that must never reach the others.
fn welcome(ws: &model::Workspace) -> Vec<live::LiveMsg> {
    let out = ws.shared_copy();
    vec![
        live::LiveMsg::Pages {
            pages: out.pages,
            deleted: out.deleted,
        },
        live::LiveMsg::Chat {
            msgs: out.chat.into_values().collect(),
        },
    ]
}

/// Merge what a peer sent into the server's copy; true when it is to be saved.
fn absorb(ws: &mut model::Workspace, msg: live::LiveMsg) -> bool {
    match msg {
        live::LiveMsg::Pages { pages, deleted } => {
            let other = model::Workspace {
                pages,
                deleted,
                ..Default::default()
            };
            collab::merge(ws, &other) > 0 || !other.deleted.is_empty()
        }
        live::LiveMsg::Chat { msgs } => ws.accept_chat(msgs) > 0,
        _ => false,
    }
}

/// `orchestra equipe …`: the team server and its settings (team.rs). Its space is kept in
/// `<data folder>/serveur-equipe/`.
fn run_team(args: &[String]) -> i32 {
    let dir = store::data_dir().join("serveur-equipe");
    let db = match team::Db::open(&dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("orchestra: espace d'équipe illisible ({}) : {e}", dir.display());
            return 1;
        }
    };
    let link = |base: &str, role: team::Role| -> i32 {
        match db.invite(role) {
            Ok(t) => {
                println!("{}/?invitation={t}", base.trim_end_matches('/'));
                0
            }
            Err(e) => {
                eprintln!("orchestra: invitation impossible : {e}");
                1
            }
        }
    };
    let here = |port: u16| format!("ws://{}:{port}", live::local_ip());
    match args.first().map(String::as_str) {
        Some("inviter") | Some("invite") => {
            let rest = args.get(1..).unwrap_or(&[]);
            let role = rest
                .iter()
                .find_map(|a| team::Role::from_key(a))
                .unwrap_or(team::Role::Member);
            let base = rest
                .iter()
                .find(|a| a.starts_with("ws://") || a.starts_with("wss://"))
                .cloned()
                .unwrap_or_else(|| here(team::DEFAULT_PORT));
            link(&base, role)
        }
        Some("ia-espace") | Some("space-ai") => {
            let a = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
            let done = match a(1) {
                "liste" | "list" | "" => {
                    for (ai, keys) in db.space_ais() {
                        println!(
                            "{}  {}  {}  {}  {keys} clé(s)",
                            ai.name,
                            ai.tool,
                            ai.base_url,
                            if ai.model.is_empty() { "(modèle par défaut)" } else { &ai.model }
                        );
                    }
                    return 0;
                }
                "ajouter" | "add" => {
                    if !matches!(a(3), "claude-code" | "codex") || a(4).is_empty() || a(2).is_empty() {
                        return usage("ia-espace ajouter <nom> <claude-code|codex> <url> [modèle] [chat]");
                    }
                    let model = if a(5) == "chat" { "" } else { a(5) };
                    let wire = if a(5) == "chat" || a(6) == "chat" { "chat" } else { "" };
                    db.set_space_ai(&space::SpaceAi {
                        name: a(2).into(),
                        tool: a(3).into(),
                        base_url: a(4).into(),
                        model: model.into(),
                        wire_api: wire.into(),
                        key: String::new(),
                    })
                    .map(|_| 1)
                }
                "cle" | "key" => {
                    if a(2).is_empty() || a(3).is_empty() || a(4).is_empty() {
                        return usage("ia-espace cle <nom> <membre|tous> <clé>");
                    }
                    if !db.space_ais().iter().any(|(x, _)| x.name == a(2)) {
                        eprintln!("orchestra: aucune IA d'espace « {} »", a(2));
                        return 1;
                    }
                    let member = if matches!(a(3), "tous" | "all") {
                        "*".to_string()
                    } else {
                        match db.find_member(a(3)) {
                            Ok(m) => m.key,
                            Err(e) => {
                                eprintln!("orchestra: {e}");
                                return 1;
                            }
                        }
                    };
                    db.set_ai_key(a(2), &member, a(4))
                }
                "retirer" | "remove" => db.remove_space_ai(a(2)),
                _ => return usage("ia-espace : ajouter, cle, retirer ou liste"),
            };
            match done {
                Ok(_) => {
                    println!("Fait. Les membres le reçoivent à leur prochaine connexion.");
                    0
                }
                Err(e) => {
                    eprintln!("orchestra: {e}");
                    1
                }
            }
        }
        Some("alleger") | Some("compact") => {
            let days: u64 = args.get(1).and_then(|d| d.parse().ok()).unwrap_or(7);
            let n = db.compact_idle(days * 24 * 3600 * 1000);
            println!("{n} page(s) allégée(s)");
            0
        }
        Some("membres") | Some("members") => {
            for m in db.member_list() {
                println!(
                    "{}  {}  {}{}",
                    &m.key[..16.min(m.key.len())],
                    m.name,
                    m.role.label(),
                    if m.revoked { "  (retiré)" } else { "" }
                );
            }
            0
        }
        Some(cmd @ ("retirer" | "remove" | "reintegrer" | "restore" | "role")) => {
            let Some(who) = args.get(1) else {
                return usage("indiquez le nom ou le début de la clé du membre");
            };
            let m = match db.find_member(who) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("orchestra: {e}");
                    return 1;
                }
            };
            let done = match cmd {
                "retirer" | "remove" => db.set_revoked(&m.key, true),
                "reintegrer" | "restore" => db.set_revoked(&m.key, false),
                _ => match args.get(2).and_then(|r| team::Role::from_key(r)) {
                    Some(r) => db.set_role(&m.key, r),
                    None => return usage("role : admin, membre ou lecteur"),
                },
            };
            match done {
                Ok(_) => {
                    println!("{} : fait", m.name);
                    0
                }
                Err(e) => {
                    eprintln!("orchestra: {e}");
                    1
                }
            }
        }
        Some("nom") | Some("name") => match args.get(1..).map(|a| a.join(" ")) {
            Some(n) if !n.trim().is_empty() => match db.set_meta("name", n.trim()) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("orchestra: {e}");
                    1
                }
            },
            _ => usage("equipe nom a besoin d'un nom"),
        },
        Some("ia") | Some("ai") => {
            let rule = match args.get(1).map(String::as_str) {
                Some("libres") | Some("free") => space::AiRule::Free,
                Some("espace") | Some("space") => space::AiRule::SpaceOnly,
                Some("aucune") | Some("none") => space::AiRule::NoAi,
                _ => return usage("equipe ia : libres, espace ou aucune"),
            };
            let allowed: Vec<String> = args.get(2..).unwrap_or(&[]).to_vec();
            match db.set_ai_rule(rule, &allowed) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("orchestra: {e}");
                    1
                }
            }
        }
        port => {
            let port = port
                .and_then(|p| p.parse().ok())
                .unwrap_or(team::DEFAULT_PORT);
            let info = db.info();
            println!("Espace « {} » sur {} ", info.name, here(port));
            println!("Données : {}", dir.display());
            if db.members() == 0 {
                print!("Première invitation (elle fera de vous l'administrateur) : ");
                if link(&here(port), team::Role::Member) != 0 {
                    return 1;
                }
            } else {
                println!("{} membre(s). Nouvelle invitation : orchestra equipe inviter", db.members());
            }
            println!(
                "Sur Internet : derrière un proxy HTTPS (Caddy, Traefik…), invitations en wss://votre-domaine ; sinon, réseau privé (Tailscale, WireGuard…)."
            );
            drop(db);
            match team::serve(&dir, port) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("orchestra: impossible d'écouter sur le port {port} : {e}");
                    1
                }
            }
        }
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
        .and_then(|v| {
            v.split_once('x')
                .and_then(|(w, h)| Some([w.trim().parse().ok()?, h.trim().parse().ok()?]))
        })
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
    match eframe::run_native(
        "Orchestra",
        options,
        Box::new(move |cc| Ok(Box::new(app::Shell::new(cc, open)))),
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("orchestra: impossible d'ouvrir la fenêtre : {e}");
            1
        }
    }
}

#[cfg(test)]
mod server_tests {
    use crate::live::LiveMsg;

    /// Someone joins the server: only the shared project comes, without its folder; never a
    /// private page or a private project's discussion.
    #[test]
    fn the_server_welcomes_with_shared_projects_only() {
        let mut ws = crate::model::Workspace::default();
        let shared = ws.create_project("Partagé", "/Users/moi/code/app");
        let private = ws.create_project("Privé", "");
        let notes = ws.create_page("Notes perso", None);
        ws.page_mut(&shared)
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .shared = true;
        for (pid, txt) in [(&shared, "salut"), (&private, "secret")] {
            ws.chat.insert(
                txt.into(),
                crate::model::ChatMsg {
                    id: txt.into(),
                    project: pid.clone(),
                    author: "moi".into(),
                    text: txt.into(),
                    at: 1,
                },
            );
        }
        let msgs = super::welcome(&ws);
        let Some(LiveMsg::Pages { pages, .. }) = msgs.first() else {
            panic!("pages first")
        };
        let ids: Vec<&str> = pages.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec![shared.as_str()]);
        assert!(!ids.contains(&notes.as_str()) && !ids.contains(&private.as_str()));
        assert!(pages[0].project.as_ref().unwrap().folder.is_empty());
        let Some(LiveMsg::Chat { msgs: chat }) = msgs.get(1) else {
            panic!("chat second")
        };
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0].text, "salut");
    }

    /// What a peer sends is merged into the copy read from the file: a page written there
    /// meanwhile (by an AI through MCP) stays; a message already known isn't saved again.
    #[test]
    fn the_server_merges_into_what_is_on_file() {
        let mut theirs = crate::model::Workspace::default();
        let p = theirs.create_project("Partagé", "");
        theirs
            .page_mut(&p)
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .shared = true;
        let mut on_file = crate::model::Workspace::default();
        let mcp = on_file.create_page("Écrite par une IA", None);
        let pages = theirs.shared_copy().pages;
        assert!(super::absorb(
            &mut on_file,
            LiveMsg::Pages {
                pages,
                deleted: Default::default()
            }
        ));
        assert!(on_file.page(&mcp).is_some() && on_file.page(&p).is_some());
        let m = crate::model::ChatMsg {
            id: "m1".into(),
            project: p.clone(),
            author: "a".into(),
            text: "salut".into(),
            at: 1,
        };
        assert!(super::absorb(
            &mut on_file,
            LiveMsg::Chat {
                msgs: vec![m.clone()]
            }
        ));
        assert!(!super::absorb(
            &mut on_file,
            LiveMsg::Chat { msgs: vec![m] }
        ));
        assert!(!super::absorb(
            &mut on_file,
            LiveMsg::Bye { name: "x".into() }
        ));
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn icon_decodes() {
        let i = super::app_icon();
        assert_eq!((i.width, i.height), (256, 256));
    }

    /// Every icon the builds point at (macOS script, Windows installer and .exe, Linux
    /// package) exists, in the right format: the ones `cargo run --example icons` makes.
    #[test]
    fn the_builds_point_at_the_new_icons() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |p: &str| std::fs::read(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
        let text = |p: &str| String::from_utf8(read(p)).unwrap();
        // macOS: the .icns copied into the app.
        assert!(text("packaging/build-macos.sh").contains("cp assets/AppIcon.icns"));
        assert_eq!(&read("assets/AppIcon.icns")[..4], b"icns");
        // Windows: the installer's icon, and the one set into orchestra.exe.
        assert!(text("packaging/orchestra.iss").contains(r"SetupIconFile=..\assets\icon.ico"));
        assert!(text(".github/workflows/release.yml").contains("--set-icon assets/icon.ico"));
        assert_eq!(&read("assets/icon.ico")[..4], &[0, 0, 1, 0]);
        // Linux: the package's icons, each a PNG of the size its folder says.
        let cargo = text("Cargo.toml");
        for (file, size) in [("assets/icon-512.png", 512), ("assets/icon-256.png", 256)] {
            assert!(
                cargo.contains(&format!("\"{file}\", \"usr/share/icons/hicolor/{size}x{size}/apps/orchestra.png\"")),
                "{file}"
            );
            let png = read(file);
            assert_eq!(&png[..4], b"\x89PNG");
            let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
            assert_eq!(w, size, "{file}");
        }
    }
}
