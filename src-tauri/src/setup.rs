//! First-run setup: look at the hardware, pick a model package, find models already on this PC,
//! and download the rest (resumable, with progress) into %LOCALAPPDATA%\Seda\models.
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub static CANCEL: AtomicBool = AtomicBool::new(false);

/// Outside the install folder (%LOCALAPPDATA%\Seda), so uninstalling or updating never deletes 5 GB of models.
pub fn models_dir() -> PathBuf {
    let d = dirs::data_local_dir().unwrap_or_default().join("SedaModels");
    let _ = std::fs::create_dir_all(&d);
    d
}

// ---------- hardware ----------

#[derive(Serialize, Clone)]
pub struct Hardware {
    pub gpu: String,
    pub vram_gb: f32,
    pub ram_gb: f32,
    pub integrated: bool,
    /// "full" | "light" | "speech"
    pub recommended: &'static str,
}

pub fn hardware() -> Hardware {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
    use windows::Win32::System::SystemInformation::{GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // Show what's physically installed (e.g. 32 GB), not what's left after the GPU reserves its share.
    let mut installed_kb = 0u64;
    let ram_gb = if unsafe { GetPhysicallyInstalledSystemMemory(&mut installed_kb) }.is_ok() && installed_kb > 0 {
        installed_kb as f32 / (1024.0 * 1024.0)
    } else {
        unsafe { GlobalMemoryStatusEx(&mut ms) }.map(|_| ms.ullTotalPhys as f32 / 1e9).unwrap_or(0.0)
    };

    // Pick the adapter with the most dedicated memory, ignoring software renderers.
    let (mut gpu, mut vram, mut shared) = (String::from("No graphics card found"), 0u64, 0u64);
    if let Ok(factory) = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() } {
        for i in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
            let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            if vram == 0 || d.DedicatedVideoMemory as u64 > vram {
                let end = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
                gpu = String::from_utf16_lossy(&d.Description[..end]).trim().to_string();
                vram = d.DedicatedVideoMemory as u64;
                shared = d.SharedSystemMemory as u64;
            }
        }
    }
    // Integrated GPUs report little dedicated memory and borrow system RAM instead.
    // AMD/Intel integrated GPUs reserve a few GB of RAM as "dedicated" but mostly borrow shared memory.
    let integrated = vram < 4_500_000_000 && shared > vram * 2;
    let recommended = if ram_gb >= 15.0 { "full" } else if ram_gb >= 7.5 { "light" } else { "speech" };
    Hardware { gpu, vram_gb: vram as f32 / 1e9, ram_gb, integrated, recommended }
}

// ---------- catalog ----------

struct Item {
    id: &'static str,
    name: &'static str,
    /// File name inside models_dir() (for zips: the file we need after unpacking).
    file: &'static str,
    url: &'static str,
    /// SHA-256 of the download (as published by its host). A mismatch is refused: a hacked or
    /// tampered host can't slip in a different program or model.
    sha256: &'static str,
    approx_mb: u32,
    /// Places this file may already exist on this PC, relative to the user's home folder.
    known: &'static [&'static str],
    zip_into: Option<&'static str>,
}

/// Persian: Whisper turbo fine-tuned on Persian (vhdm, MIT); 1.7% CER in our bake-off.
const SPEECH_FA: Item = Item {
    id: "speech",
    name: "Persian speech model",
    file: "ggml-fa-turbo-q5_0.bin",
    url: "https://github.com/d1same/seda-models/releases/download/v1/ggml-fa-turbo-q5_0.bin",
    sha256: "6debcd339f7504abd0e5328776900bf4bda67280353f5754277684ff7626dec1",
    approx_mb: 548,
    known: &[],
    zip_into: None,
};
/// Every other language: OpenAI's Whisper large-v3-turbo, as published by the whisper.cpp project.
const SPEECH_MULTI: Item = Item {
    id: "speech",
    name: "Speech model (multilingual)",
    file: "ggml-large-v3-turbo-q5_0.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin",
    sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    approx_mb: 548,
    known: &[],
    zip_into: None,
};
const VAD: Item = Item {
    id: "vad",
    name: "Voice detector",
    file: "ggml-silero-v6.2.0.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    approx_mb: 1,
    known: &[],
    zip_into: None,
};
const SERVER: Item = Item {
    id: "server",
    name: "AI engine (llama.cpp, GPU)",
    file: r"llama\llama-server.exe",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11819/llama-b11819-bin-win-vulkan-x64.zip",
    sha256: "c3b2484d18258425fe3f69d0b6d07dce01501d6e010578dad3e53150a30df557",
    approx_mb: 35,
    known: &[],
    zip_into: Some("llama"),
};
const LLM_FULL: Item = Item {
    id: "llm",
    name: "Writing AI (Gemma 4 E4B)",
    file: "gemma-4-E4B-it-Q4_K_M.gguf",
    url: "https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF/resolve/bfc15c382204943c3a8fff0c750b94ae2364d7a3/gemma-4-E4B-it-Q4_K_M.gguf",
    sha256: "85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87",
    approx_mb: 4600,
    known: &[],
    zip_into: None,
};
const LLM_LIGHT: Item = Item {
    id: "llm",
    name: "Writing AI (Gemma 4 E2B, lighter)",
    file: "gemma-4-E2B-it-Q4_K_M.gguf",
    url: "https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/0314792d7f1f7e229411f620751375812bb9faf2/gemma-4-E2B-it-Q4_K_M.gguf",
    sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8",
    approx_mb: 2900,
    known: &[],
    zip_into: None,
};

fn existing(item: &Item) -> Option<PathBuf> {
    let local = models_dir().join(item.file);
    if local.exists() {
        return Some(local);
    }
    let home = dirs::home_dir()?;
    item.known.iter().map(|k| home.join(k)).find(|p| p.exists())
}

/// Persian gets its fine-tuned model; every other language the multilingual one.
fn speech_item(language: &str) -> &'static Item {
    if language == "fa" { &SPEECH_FA } else { &SPEECH_MULTI }
}

/// Path of the speech model for `language` if it is already on this PC.
pub fn speech_model_for(language: &str) -> Option<String> {
    existing(speech_item(language)).map(|p| p.to_string_lossy().into_owned())
}

fn package(pack: &str, language: &str) -> Vec<&'static Item> {
    let mut items = vec![speech_item(language), &VAD];
    match pack {
        "full" => items.extend([&SERVER, &LLM_FULL]),
        "light" => items.extend([&SERVER, &LLM_LIGHT]),
        _ => {}
    }
    items
}

#[derive(Serialize, Clone)]
pub struct ItemStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub size_mb: u32,
    /// "ready" (found on this PC) | "missing"
    pub state: &'static str,
    pub path: String,
}

pub fn status(pack: &str, language: &str) -> Vec<ItemStatus> {
    package(pack, language)
        .into_iter()
        .map(|it| {
            let found = existing(it);
            ItemStatus {
                id: it.id,
                name: it.name,
                size_mb: it.approx_mb,
                state: if found.is_some() { "ready" } else { "missing" },
                path: found.unwrap_or_else(|| models_dir().join(it.file)).to_string_lossy().into_owned(),
            }
        })
        .collect()
}

/// Paths to use once everything is present: (speech, llm server, llm model).
pub fn resolved_paths(pack: &str, language: &str) -> (String, String, String) {
    let p = |it: &Item| existing(it).unwrap_or_else(|| models_dir().join(it.file)).to_string_lossy().into_owned();
    let llm = if pack == "light" { &LLM_LIGHT } else { &LLM_FULL };
    (p(speech_item(language)), p(&SERVER), p(llm))
}

/// Make the configured model paths valid on *this* PC: replace any that don't exist with a model
/// found locally. Returns false when the speech model is missing, so setup must run again.
pub fn reconcile(s: &mut crate::settings::Settings) -> bool {
    // settings.json is a plain file: never run or load anything from outside the models folder.
    for (path, engine) in [(&mut s.whisper_model, false), (&mut s.llm_model, false), (&mut s.llm_server, true)] {
        if !path.is_empty() && !is_model_path(path, engine) {
            crate::log::log("ignored a model/engine path outside the models folder");
            path.clear();
        }
    }
    let exists = |p: &str| !p.is_empty() && Path::new(p).exists();
    // The speech model must match the chosen language (Persian model vs multilingual).
    let wanted = speech_item(&s.language).file;
    if !exists(&s.whisper_model) || !s.whisper_model.ends_with(wanted) {
        if let Some(p) = existing(speech_item(&s.language)) {
            s.whisper_model = p.to_string_lossy().into_owned();
        } else {
            s.whisper_model = String::new();
        }
    }
    if !exists(&s.llm_server) {
        if let Some(p) = existing(&SERVER) {
            s.llm_server = p.to_string_lossy().into_owned();
        }
    }
    if !exists(&s.llm_model) {
        if let Some(p) = existing(&LLM_FULL).or_else(|| existing(&LLM_LIGHT)) {
            s.llm_model = p.to_string_lossy().into_owned();
        }
    }
    let writing_ok = !s.cleanup || (exists(&s.llm_server) && exists(&s.llm_model));
    exists(&s.whisper_model) && writing_ok
}

/// True if path is an existing file inside the models folder (for the engine: llama-server.exe).
pub fn is_model_path(path: &str, engine: bool) -> bool {
    if path.starts_with(r"\\") || path.starts_with("//") {
        return false; // network path
    }
    let (Ok(p), Ok(root)) = (std::fs::canonicalize(path), std::fs::canonicalize(models_dir())) else { return false };
    let name_ok = !engine || p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("llama-server.exe"));
    p.is_file() && p.starts_with(&root) && name_ok
}

/// Where the voice detector lives (next to the speech model, or in the models folder).
pub fn vad_path(speech_model: &str) -> PathBuf {
    let beside = Path::new(speech_model).with_file_name(VAD.file);
    if beside.exists() { beside } else { existing(&VAD).unwrap_or(beside) }
}

// ---------- download ----------

/// Download every missing item of `pack`. `progress(id, done_bytes, total_bytes)`.
pub fn download_missing(pack: &str, language: &str, progress: &dyn Fn(&str, u64, u64)) -> Result<()> {
    CANCEL.store(false, Ordering::SeqCst);
    for it in package(pack, language) {
        if existing(it).is_some() {
            continue;
        }
        if it.url.is_empty() {
            bail!("{} has no download source", it.name);
        }
        let dir = models_dir();
        let target = match it.zip_into {
            Some(_) => dir.join(format!("{}.zip", it.id)),
            None => dir.join(it.file),
        };
        crate::log::log(format!("downloading {} from {}", it.name, it.url));
        fetch(it.url, &target, &|d, t| progress(it.id, d, t)).with_context(|| format!("Downloading {}", it.name))?;
        verify(&target, it.sha256).with_context(|| format!("Checking {}", it.name))?;
        if let Some(sub) = it.zip_into {
            unzip(&target, &dir.join(sub))?;
            let _ = std::fs::remove_file(&target);
            if !dir.join(it.file).exists() {
                bail!("{} was not in the downloaded archive", it.file);
            }
        }
        crate::log::log(format!("downloaded {}", it.name));
    }
    Ok(())
}

fn fetch(url: &str, dest: &Path, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let part = dest.with_file_name(format!("{}.part", dest.file_name().unwrap().to_string_lossy()));
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(None)
        .user_agent("Seda/0.1")
        .build()?;
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-")); // resume
    }
    let mut resp = req.send()?.error_for_status()?;
    let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut done = if resumed { have } else { 0 };
    let total = done + resp.content_length().unwrap_or(0);
    let mut f = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut last = Instant::now();
    progress(done, total);
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            bail!("Cancelled");
        }
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(150) {
            progress(done, total);
            last = Instant::now();
        }
    }
    f.flush()?;
    drop(f);
    progress(done, total);
    if total > 0 && done != total {
        bail!("Download incomplete ({done} of {total} bytes) — try again to resume");
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// Refuse a download whose SHA-256 isn't the published one (and delete it, so it's never used).
fn verify(path: &Path, expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    let got = format!("{:x}", hasher.finalize());
    if !got.eq_ignore_ascii_case(expected) {
        let _ = std::fs::remove_file(path);
        crate::log::log(format!("download check failed for {}: expected {expected}, got {got}", path.display()));
        bail!("The download didn't match its official fingerprint and was deleted. Try again; if it keeps failing, the download source may have been tampered with.");
    }
    Ok(())
}

/// Command-line check of the downloader: fetch + unpack the AI engine into `dir`.
pub fn download_test(dir: &Path) -> Result<()> {
    let zip = dir.join("server.zip");
    fetch(SERVER.url, &zip, &|d, t| print!("\r{:.1} / {:.1} MB   ", d as f64 / 1e6, t as f64 / 1e6))?;
    println!();
    verify(&zip, SERVER.sha256)?;
    println!("fingerprint OK");
    unzip(&zip, &dir.join("llama"))?;
    println!("unpacked: llama-server.exe present = {}", dir.join(SERVER.file).exists());
    Ok(())
}

fn unzip(zip_path: &Path, into: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    std::fs::create_dir_all(into)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        // The release zips put files either at the root or in one folder; flatten to `into`.
        let name = rel.file_name().ok_or_else(|| anyhow!("bad zip entry"))?;
        if entry.is_dir() {
            continue;
        }
        let mut out = std::fs::File::create(into.join(name))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}
