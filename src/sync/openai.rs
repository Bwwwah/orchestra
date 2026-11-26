//! A `Provider` for any model behind an OpenAI-compatible address
//! (`POST <address>/chat/completions`): Ollama and the other servers running a model on this
//! computer (LM Studio, llama.cpp, vLLM…), or Mistral's own service.
//!
//! The model gets the prompt and the images, nothing else: it has no tool, so it can't read
//! the project's folder (what it must know has to be in the prompt, see `brief`), and it
//! keeps no conversation to carry on.
//!
//! The call goes through `curl`, everything handed over on its standard input: neither the
//! key nor the prompt shows on a command line or lands in a temporary file.

use super::provider::{Provider, Reply, Request};
use crate::model::TextModel;
use base64::Engine;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Largest image sent to a model (bytes).
const MAX_IMAGE: u64 = 10 * 1024 * 1024;

/// Which kind of server answers: it sets the usual address, the name shown and the key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Service {
    /// Ollama on this computer: no key.
    Ollama,
    /// Mistral's service: needs a key.
    Mistral,
    /// Any other compatible server: a key only if it asks for one.
    Other,
}

/// How much of a project a model on this computer is told (characters): servers such as
/// Ollama often give a model a window of 4096 tokens, answer included, and cut what is over
/// without saying so.
const LOCAL_BRIEF: usize = 8_000;
/// The same for a model on a server elsewhere, whose window is far larger.
const REMOTE_BRIEF: usize = 60_000;

impl Service {
    pub const ALL: [Service; 3] = [Service::Ollama, Service::Mistral, Service::Other];

    /// The name it is saved under in the settings, and its provider id.
    pub fn id(self) -> &'static str {
        match self {
            Service::Ollama => "ollama",
            Service::Mistral => "mistral",
            Service::Other => "openai-compatible",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Service::Ollama => "Ollama",
            Service::Mistral => "Mistral",
            Service::Other => crate::tr!("Modèle compatible OpenAI", "OpenAI-compatible model"),
        }
    }

    /// The address a request goes to, from the one the person typed ("": the usual one).
    /// Mistral's is always its own, so that its key never goes anywhere else.
    fn address(self, typed: &str) -> &str {
        match typed.trim() {
            u if !u.is_empty() && self != Service::Mistral => u,
            _ => self.default_url(),
        }
    }

    /// The usual address ("": the person has to give it).
    pub fn default_url(self) -> &'static str {
        match self {
            Service::Ollama => "http://localhost:11434/v1",
            Service::Mistral => "https://api.mistral.ai/v1",
            Service::Other => "",
        }
    }

    /// The name its key is kept under (`secret::api_key`). Never another service's key, so
    /// an address never receives a key meant for someone else.
    pub fn key_var(self) -> Option<&'static str> {
        match self {
            Service::Ollama => None,
            Service::Mistral => Some("MISTRAL_API_KEY"),
            Service::Other => Some("OPENAI_COMPATIBLE_API_KEY"),
        }
    }
}

#[derive(Clone)]
pub struct OpenAi {
    service: Service,
    /// Without its last slash.
    url: String,
    model: String,
    /// "": none is sent.
    key: String,
}

impl OpenAi {
    /// The provider for `model` at `url` ("": the service's usual address), with the key the
    /// Keychain keeps for this service.
    pub fn new(service: Service, url: &str, model: &str) -> Self {
        let key = service
            .key_var()
            .map(crate::secret::api_key)
            .unwrap_or_default();
        Self::with_key(service, url, model, &key)
    }

    /// The model the settings name; None when they leave the work to Claude Code or Codex.
    pub fn chosen(choice: &TextModel) -> Option<Self> {
        Service::from_id(&choice.service).map(|s| Self::new(s, &choice.url, &choice.model))
    }

    fn with_key(service: Service, url: &str, model: &str, key: &str) -> Self {
        OpenAi {
            service,
            url: service.address(url).trim_end_matches('/').to_string(),
            model: model.trim().to_string(),
            key: key.trim().to_string(),
        }
    }

    /// How much of a project to put in a question (characters), see `brief`.
    pub fn brief_budget(&self) -> usize {
        match crate::proxy::upstream(&self.url) {
            Some(u) if is_this_computer(&u.1) => LOCAL_BRIEF,
            _ => REMOTE_BRIEF,
        }
    }

    /// What stops a request before anything is sent.
    pub fn check(&self) -> Result<(), String> {
        let invalid = || {
            crate::trf!(
                "Adresse du modèle invalide : « {} » (attendu : http://… ou https://…).",
                "Invalid model address: « {} » (expected: http://… or https://…).",
                self.url
            )
        };
        if self
            .url
            .contains(|c: char| c.is_whitespace() || c.is_control() || "@?#\"\\".contains(c))
        {
            return Err(invalid());
        }
        let (tls, host, ..) = crate::proxy::upstream(&self.url).ok_or_else(invalid)?;
        if self.model.is_empty() {
            return Err(crate::tr!("Aucun modèle choisi.", "No model chosen.").into());
        }
        if self.service == Service::Mistral && self.key.is_empty() {
            return Err(crate::tr!(
                "Il manque la clé API de Mistral.",
                "Mistral's API key is missing."
            )
            .into());
        }
        if !self.key.is_empty() && !tls && !is_this_computer(&host) {
            return Err(crate::tr!(
                "La clé API ne part pas en clair sur le réseau : utilisez une adresse https://.",
                "The API key isn't sent in clear over the network: use an https:// address."
            )
            .into());
        }
        Ok(())
    }
}

fn is_this_computer(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The picture at `path` (relative to `folder`) as a `data:` address.
fn data_url(folder: &str, path: &str) -> Result<String, String> {
    let file = crate::project::normalize_folder(folder).join(path);
    let unreadable = || {
        crate::trf!(
            "Image illisible ou de plus de 10 Mo : {}",
            "Image unreadable or over 10 MB: {}",
            path
        )
    };
    let kind = match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "png",
        Some("jpg" | "jpeg") => "jpeg",
        Some("gif") => "gif",
        Some("webp") => "webp",
        _ => return Err(unreadable()),
    };
    if !std::fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() <= MAX_IMAGE) {
        return Err(unreadable());
    }
    let bytes = std::fs::read(&file).map_err(|_| unreadable())?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:image/{kind};base64,{data}"))
}

/// The JSON sent: one message, the text alone or followed by the images.
fn body(model: &str, request: &Request) -> Result<String, String> {
    let content = if request.images.is_empty() {
        json!(request.prompt)
    } else {
        let mut parts = vec![json!({ "type": "text", "text": request.prompt })];
        for image in request.images {
            let url = data_url(request.folder, image)?;
            parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
        }
        Value::Array(parts)
    };
    Ok(json!({
        "model": model,
        "messages": [{ "role": "user", "content": content }],
        "stream": false,
    })
    .to_string())
}

/// A value between quotes in a `curl` config file. JSON has no raw line break, so only the
/// backslash and the quote need escaping; anything else that would end the line is refused
/// by the callers (`check`) or removed here.
fn quoted(value: &str) -> String {
    let v: String = value.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

fn curl_config(url: &str, key: &str, body: &str) -> String {
    let mut c = format!(
        "url = {}\nheader = \"Content-Type: application/json\"\ndata-binary = {}\n",
        quoted(&format!("{url}/chat/completions")),
        quoted(body)
    );
    if !key.is_empty() {
        c.push_str(&format!(
            "header = {}\n",
            quoted(&format!("Authorization: Bearer {key}"))
        ));
    }
    c
}

/// The text of the model's message: a string, or a list of parts of which only the text ones
/// count (a reasoning model's thinking comes apart, or first between `<think>` marks).
fn message_text(content: &Value) -> String {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect(),
        _ => String::new(),
    };
    let text = text.trim();
    match text.strip_prefix("<think>") {
        // Cut off while still thinking: there is no answer.
        Some(rest) => rest
            .split_once("</think>")
            .map_or("", |(_, answer)| answer)
            .trim()
            .to_string(),
        None => text.to_string(),
    }
}

/// What the server said went wrong, in whichever of the usual shapes.
fn server_message(body: &str) -> String {
    let said = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        [&v["error"]["message"], &v["error"], &v["message"], &v["detail"]]
            .into_iter()
            .find_map(|m| m.as_str().map(String::from))
    });
    crate::richtext::one_line(said.as_deref().unwrap_or(body).trim(), 160)
}

/// The answer in a server's response: (text, cut short for lack of room).
fn read_answer(status: u16, body: &str) -> Result<(String, bool), String> {
    let said = server_message(body);
    match status {
        200 => {}
        401 | 403 => {
            return Err(crate::trf!(
                "Le serveur du modèle refuse la clé API ({}).",
                "The model's server refuses the API key ({}).",
                said
            ));
        }
        _ => {
            return Err(crate::trf!(
                "Le serveur du modèle répond une erreur {} : {}",
                "The model's server answers error {}: {}",
                status,
                said
            ));
        }
    }
    let unreadable = || {
        crate::tr!(
            "Réponse du modèle illisible : l'adresse est-elle bien compatible OpenAI ?",
            "The model's answer is unreadable: is the address OpenAI-compatible?"
        )
        .to_string()
    };
    let v: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    let choice = &v["choices"][0];
    if !choice["message"].is_object() {
        return Err(unreadable());
    }
    Ok((
        message_text(&choice["message"]["content"]),
        choice["finish_reason"] == "length",
    ))
}

impl Provider for OpenAi {
    fn id(&self) -> &str {
        self.service.id()
    }

    fn label(&self) -> &str {
        self.service.label()
    }

    fn run(&self, request: &Request, cancel: &AtomicBool) -> Result<Reply, String> {
        if request.resume.is_some() {
            return Err(super::cannot_resume().into());
        }
        self.check()?;
        let body = body(&self.model, request)?;
        let path = super::shell_path();
        let curl = super::which("curl", &path)
            .ok_or(crate::tr!("« curl » introuvable.", "« curl » not found."))?;
        let mut cmd = crate::proc::command(curl);
        // `-q` first: the person's .curlrc doesn't apply. No redirect is followed, so the
        // key only ever goes to the address given.
        cmd.args(["-q", "-sS", "--globoff", "--proto", "=http,https"])
            .args(["--connect-timeout", "10", "-w", "\n%{http_code}", "-K", "-"]);
        // A proxy set in the environment must not stand between us and this computer.
        if crate::proxy::upstream(&self.url).is_some_and(|u| is_this_computer(&u.1)) {
            cmd.args(["--noproxy", "*"]);
        }
        let mut child = cmd
            .env("PATH", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        // Pipes in threads: neither side can block on a full one, and `cancel` stays heard.
        let config = curl_config(&self.url, &self.key, &body);
        let stdin = child.stdin.take();
        std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(config.as_bytes());
            }
        });
        let read = |r: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = vec![];
                if let Some(mut r) = r {
                    let _ = r.read_to_end(&mut bytes);
                }
                String::from_utf8_lossy(&bytes).into_owned()
            })
        };
        let out = read(child.stdout.take().map(|x| Box::new(x) as Box<dyn Read + Send>));
        let err = read(child.stderr.take().map(|x| Box::new(x) as Box<dyn Read + Send>));
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            let stopped = cancel.load(Ordering::Relaxed);
            if stopped || start.elapsed() > request.limit {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if stopped {
                    crate::tr!("arrêté", "stopped").into()
                } else {
                    crate::tr!("trop long, arrêté", "too long, stopped").into()
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let out = out.join().unwrap_or_default();
        let err = err.join().unwrap_or_default();
        if !status.success() {
            return Err(crate::trf!(
                "Modèle injoignable à l'adresse {} : est-il lancé ? ({})",
                "No model reachable at {}: is it running? ({})",
                self.url,
                super::last_line(&err).unwrap_or_default()
            ));
        }
        let (body, code) = out.rsplit_once('\n').unwrap_or(("", &out));
        let (text, cut) = read_answer(code.trim().parse().unwrap_or(0), body)?;
        let secs = start.elapsed().as_secs();
        let mut report = crate::trf!("terminé en {} s", "done in {} s", secs);
        if cut {
            report.push_str(crate::tr!(", réponse coupée", ", answer cut short"));
        }
        Ok(Reply {
            text,
            report,
            session: None,
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::net::TcpListener;

    fn ask(provider: &OpenAi, request: &Request) -> Result<Reply, String> {
        provider.run(request, &AtomicBool::new(false))
    }

    /// A server that takes one request, gives it back through the channel and answers
    /// `status` with `answer`. Returns its address.
    pub(in crate::sync) fn server(
        status: &'static str,
        answer: &'static str,
    ) -> (String, std::sync::mpsc::Receiver<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut got = vec![];
            let mut buf = [0u8; 4096];
            let (head, body) = loop {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "request cut short");
                got.extend_from_slice(&buf[..n]);
                let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&got[..end]).into_owned();
                let parsed = crate::proxy::parse_head(&got[..end]).unwrap();
                // curl waits for this before a large body.
                if parsed.header("expect").is_some() && got.len() == end + 4 {
                    s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
                }
                let len: usize = parsed.header("content-length").unwrap().parse().unwrap();
                if got.len() >= end + 4 + len {
                    break (head, String::from_utf8(got[end + 4..].to_vec()).unwrap());
                }
            };
            tx.send((head, body)).unwrap();
            write!(
                s,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            )
            .unwrap();
        });
        (url, rx)
    }

    const HELLO: &str =
        r#"{"choices":[{"message":{"role":"assistant","content":"Bonjour"},"finish_reason":"stop"}]}"#;

    #[test]
    fn the_question_reaches_the_server_as_written() {
        let (url, rx) = server("200 OK", HELLO);
        let provider = OpenAi::with_key(Service::Other, &url, "mistral:7b", "sk-\"secret\\");
        // Quotes, backslashes, line breaks and a long text all come through curl untouched.
        let prompt = format!("@fichier \"a\\b\"\n\tété {}", "x".repeat(200_000));
        let reply = ask(&provider, &Request::new("/tmp", &prompt)).unwrap();
        assert_eq!(reply.text, "Bonjour");
        assert_eq!(reply.session, None);
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with("POST /v1/chat/completions HTTP/1.1"), "{head}");
        assert!(head.contains("Authorization: Bearer sk-\"secret\\\r\n"), "{head}");
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent["model"], "mistral:7b");
        assert_eq!(sent["stream"], false);
        assert_eq!(sent["messages"][0]["content"], prompt.as_str());
    }

    #[test]
    fn images_go_with_the_text_and_no_key_means_no_header() {
        let dir = std::env::temp_dir().join(format!("orchestra-openai-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.PNG"), b"abc").unwrap();
        let (url, rx) = server("200 OK", HELLO);
        let provider = OpenAi::with_key(Service::Ollama, &url, "llava", "");
        let folder = dir.display().to_string();
        let images = ["a.PNG".to_string()];
        let request = Request {
            images: &images,
            ..Request::new(&folder, "Que vois-tu ?")
        };
        ask(&provider, &request).unwrap();
        let (head, body) = rx.recv().unwrap();
        assert!(!head.to_lowercase().contains("authorization"), "{head}");
        let sent: Value = serde_json::from_str(&body).unwrap();
        let parts = &sent["messages"][0]["content"];
        assert_eq!(parts[0]["text"], "Que vois-tu ?");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,YWJj");
        let missing = ["b.png".to_string()];
        let request = Request {
            images: &missing,
            ..Request::new(&folder, "?")
        };
        assert!(ask(&provider, &request).unwrap_err().contains("b.png"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn errors_say_what_happened() {
        let (url, _rx) = server("404 Not Found", r#"{"error":{"message":"model 'x' not found"}}"#);
        let provider = OpenAi::with_key(Service::Ollama, &url, "x", "");
        let error = ask(&provider, &Request::new("/tmp", "?")).unwrap_err();
        assert!(error.contains("404") && error.contains("model 'x' not found"), "{error}");
        let (url, _rx) = server("401 Unauthorized", r#"{"message":"Unauthorized"}"#);
        let provider = OpenAi::with_key(Service::Mistral, &url, "m", "k");
        assert!(ask(&provider, &Request::new("/tmp", "?")).unwrap_err().contains("API"));
        // Nobody listens: the address is named.
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let url = format!("http://{free}/v1");
        let provider = OpenAi::with_key(Service::Ollama, &url, "x", "");
        assert!(ask(&provider, &Request::new("/tmp", "?")).unwrap_err().contains(&url));
    }

    #[test]
    fn a_silent_server_is_given_up_on() {
        // Accepts the connection (the system does) and never answers.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let provider = OpenAi::with_key(Service::Ollama, &url, "x", "");
        let request = Request {
            limit: Duration::from_millis(500),
            ..Request::new("/tmp", "?")
        };
        let late = ask(&provider, &request).unwrap_err();
        let stopped = provider.run(&Request::new("/tmp", "?"), &AtomicBool::new(true));
        assert_ne!(late, stopped.unwrap_err());
    }

    #[test]
    fn nothing_is_sent_when_the_setup_is_wrong() {
        let check = |service, url: &str, model: &str, key: &str| {
            OpenAi::with_key(service, url, model, key).check().is_ok()
        };
        assert!(check(Service::Ollama, "", "qwen3", ""));
        assert!(check(Service::Mistral, "", "mistral-small-latest", "k"));
        assert!(!check(Service::Mistral, "", "mistral-small-latest", ""));
        assert!(!check(Service::Ollama, "", " ", ""));
        assert!(!check(Service::Other, "", "m", ""));
        for url in ["file:///etc/passwd", "localhost:11434", "http://localhost@evil.example/v1", "http://a b/v1"] {
            assert!(!check(Service::Other, url, "m", ""), "{url}");
        }
        // A key never leaves this computer in clear.
        for url in ["http://localhost:1234/v1", "http://127.0.0.1/v1", "http://[::1]:8000/v1", "https://ia.example/v1"] {
            assert!(check(Service::Other, url, "m", "k"), "{url}");
        }
        for url in ["http://ia.example/v1", "http://10.0.0.5:4000", "http://localhost.evil.example"] {
            assert!(!check(Service::Other, url, "m", "k"), "{url}");
            assert!(check(Service::Other, url, "m", ""), "{url}");
        }
        let provider = OpenAi::with_key(Service::Ollama, "", "m", "");
        let request = Request {
            resume: Some("abc"),
            ..Request::new("/tmp", "?")
        };
        assert!(ask(&provider, &request).is_err());
    }

    #[test]
    fn the_settings_name_the_model_and_its_address() {
        let choice = |service: &str, url: &str| TextModel {
            service: service.into(),
            url: url.into(),
            model: "m".into(),
        };
        // Nothing chosen, or a service this version doesn't know: Claude Code or Codex.
        assert!(OpenAi::chosen(&TextModel::default()).is_none());
        assert!(OpenAi::chosen(&choice("autre-chose", "")).is_none());
        for service in Service::ALL {
            assert_eq!(Service::from_id(service.id()), Some(service));
        }
        // Mistral's key only goes to Mistral, whatever address was left in the settings.
        let mistral = OpenAi::with_key(Service::Mistral, "https://ailleurs.example/v1", "m", "k");
        assert_eq!(mistral.url, Service::Mistral.default_url());
        assert_eq!(mistral.brief_budget(), REMOTE_BRIEF);
        let local = OpenAi::chosen(&choice("ollama", "")).unwrap();
        assert_eq!(local.url, Service::Ollama.default_url());
        assert_eq!(local.brief_budget(), LOCAL_BRIEF);
        let other = OpenAi::with_key(Service::Other, " http://127.0.0.1:1234/v1/ ", "m", "");
        assert_eq!(other.url, "http://127.0.0.1:1234/v1");
        assert_eq!(other.brief_budget(), LOCAL_BRIEF);
    }

    #[test]
    fn answers_are_read_in_their_usual_shapes() {
        let answer = |content: Value| message_text(&content);
        assert_eq!(answer(json!("<think>hum</think>\n\nOui")), "Oui");
        assert_eq!(answer(json!("<think>jamais fini")), "");
        assert_eq!(
            answer(json!([{"type":"thinking","thinking":[]},{"type":"text","text":"Oui"}])),
            "Oui"
        );
        assert_eq!(answer(Value::Null), "");
        let cut = r#"{"choices":[{"message":{"content":"Ou"},"finish_reason":"length"}]}"#;
        assert_eq!(read_answer(200, cut), Ok(("Ou".into(), true)));
        assert!(read_answer(200, "<html>").is_err());
        assert!(read_answer(200, "{}").is_err());
        assert!(read_answer(500, r#"{"error":"panne"}"#).unwrap_err().contains("panne"));
    }
}
