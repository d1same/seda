<p align="center"><img src="src-tauri/icons/seda-icon.svg" width="96" alt="Seda"></p>

# Seda

Private voice typing and meeting notes for Windows. Everything runs on your own computer: nothing you say is uploaded.

**[Download the installer](https://github.com/d1same/seda/releases/latest)** (Windows 10/11, 64-bit). That file is what other people install. They run it; they do not need this repository.

- **Dictation anywhere.** Hold a shortcut (any key or mouse button, e.g. Mouse 4) and talk. The capsule shows the words while you are still holding. Let go and they paste at the cursor right away. Say "no wait…" and Seda drops the part you took back.
- **English + one more language.** Mix them in the same sentence. Persian has its own fine-tuned speech model; Chinese, Hindi, Spanish, Arabic, French, Portuguese, Russian, German, Japanese and Greek use the multilingual one.
- **Writing AI.** After the words land, a local model can tidy that same sentence in place: punctuation, paragraphs, and the app you are in (email layout in Outlook/Gmail, bullets in Jira, casual in Teams/Slack). It also sees the text already in the box, so "reply yes, and copy Bob" can use the sentence on screen.
- **Snippets.** Say "my email" and Seda pastes the address from Settings. Add your own phrase → text pairs there too.
- **Fix my text.** Select text anywhere, press a shortcut: correct grammar, make it formal, friendly, shorter, or translate.
- **Meetings.** Records you (microphone) and everyone else (the computer's audio), transcribes live, and writes notes with decisions and action items when the call ends. No bot joins your call. Seda notices when Teams, Zoom, Meet and others start a call and offers to take notes. If Outlook is already open, the meeting takes the appointment's name, and one other person is named in the transcript. Search also matches what was said.
- **Ask about a meeting.** "What did I agree to do?", "Write a follow-up email", "Make the notes shorter": answers and edits come from the local AI.
- **Learns from you.** Correct a dictation once and Seda remembers the fix; add names and terms it should always spell right.

## How it works

| Part | What runs locally |
|---|---|
| Speech to text | [whisper.cpp](https://github.com/ggerganov/whisper.cpp) (Vulkan GPU), large-v3-turbo; a Persian fine-tune for Persian |
| Voice detection | Silero VAD |
| Writing AI | [llama.cpp](https://github.com/ggerganov/llama.cpp) server with Gemma 4 (E4B or E2B, picked by your PC's memory) |
| App | [Tauri 2](https://tauri.app) (Rust) + React |

The first run downloads the models (about 5 GB for the full package, less for the light ones) into `%LOCALAPPDATA%\SedaModels`. Your settings, history and meetings stay in `%APPDATA%\Seda`.

## Build from source

Requirements (Windows 10/11, x64):

1. [Rust](https://rustup.rs) (MSVC toolchain) and [Node.js](https://nodejs.org) 20+
2. Visual Studio 2022 Build Tools (C++ workload, includes CMake)
3. [LLVM](https://github.com/llvm/llvm-project/releases) (for `libclang`)
4. [Vulkan SDK](https://vulkan.lunarg.com/sdk/home)

Check the tool paths in `src-tauri/.cargo/config.toml`, then paste this. A new window refuses scripts until the second line. That includes `npm`, which Windows runs as `npm.ps1`.

```powershell
cd Z:\Projects\Whisper\seda
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
npm install
npm run tauri dev
```

`npm run dev` alone opens the interface in a browser with sample data. A release installer is the publish command below. `npm run tauri build -- --no-sign` writes an unsigned installer at `Z:\Projects\Whisper\b\release\bundle\nsis\` and keeps the password question out of the build log.

## Publish a release

0.2.10 is already published: https://github.com/d1same/seda/releases/tag/v0.2.10. The next release uses the next version number.

Write the real change in `notes.md` before you run anything. That sentence is what installed copies show. Then paste this:

```powershell
cd Z:\Projects\Whisper\seda
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\scripts\release.ps1 -Version 0.2.11 -Notes notes.md
```

The script sets `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml` to the version you pass. The build takes several minutes. When the window says to expect a password, press Enter. The password is empty. GitHub gets the installer and `latest.json`, and a copy is saved in `Z:\Projects\Whisper\installer`. The release tag is the commit that was just built, so the download link above serves that installer.

You should see `Set package.json to 0.2.11.` and later `Press Enter at the password question.` A cursor sitting on the command line means that command has not been submitted yet.

If the installer for that version is already built, sign and publish that file:

```powershell
cd Z:\Projects\Whisper\seda
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\scripts\release.ps1 -Version 0.2.11 -Notes notes.md -SkipBuild
```

`-SkipBuild` prints `Signing the installer that is already built.` immediately.

Compiler files stay in `Z:\Projects\Whisper\b`. A release build's shader tool uses `Z:\Projects\Whisper\shader`. `npm run tauri dev` uses `Z:\Projects\Whisper\shader-dev`, so the two builds do not share one CMake cache. Both sit next to the repo, so `Z:\Projects\Seda\seda` works the same way after the parent folder is renamed. The signing key is `%USERPROFILE%\.tauri\seda-updater.key`. `gh auth status` has to show you logged in.

### If the window shows one of these

These are the stops from the 0.2.10 publish. The current script already avoids them. `git pull` in `Z:\Projects\Whisper\seda` on branch `cursor/clickable-idle-dot-dabd` brings that script in.

| What you see | What to do |
|---|---|
| `.\scripts\release.ps1` is not recognized | The window is outside the project. Start with `cd Z:\Projects\Whisper\seda`. |
| Running scripts is disabled, or `npm.ps1 cannot be loaded` | `Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass` in that window, then run the command again. It lasts until the window closes. Winget's `npm` is a script, so `npm run tauri dev` needs this too. |
| `package.json is not at version` | That is an old copy of the script. `git pull`, then run the command again. The current script writes the version. |
| `notes.md still has the example sentence` | `notes.md` is still a sample. Replace it with the real change, then run the command again. |
| `v0.2.10 is already published` | That version is on GitHub. Pass the next number, such as `0.2.11`. |
| `gh : release not found` | That version is free. `git pull`, then run the publish command again. The check was stopping the script. |
| Enter does nothing at `expect a prompt for password` | The password question was trapped in the build log. Pull the current script. It builds with `--no-sign`, then asks in this window. Press Enter. |
| `a value is required for '--password'` | An empty `-p ""` was passed. Windows PowerShell drops that. The current script asks in the window and does not pass `-p`. |
| `--private-key-path cannot be used with --private-key` | The key path was in `TAURI_SIGNING_PRIVATE_KEY` and also passed as a file. The script clears that variable before it signs. |
| `git pull` says Already up to date, then the old error remains | The window is on an older commit. `git status` should show `cursor/clickable-idle-dot-dabd`, then `git pull`. |
| `Run this from Z:\Projects\Whisper\seda` | This copy is outside the project. `Z:\Projects\Seda\seda` is accepted after a rename. |
| `Signing key not found` | The key stays at `%USERPROFILE%\.tauri\seda-updater.key`. |
| `Tauri's command was not found` | Run `npm install` in the project folder. |
| `publishing failed` after a finished build | The tag already exists, or `gh` is logged out. `gh auth status`, then publish the next version with `-SkipBuild`. |
| A path-length or `FTK1011` / `MSB6003` error on a `.tlog` | The shader tool builds in `Z:\Projects\Whisper\shader` for a release, and `shader-dev` for `npm run tauri dev`. Leave compiler files in `Z:\Projects\Whisper\b`. |
| `does not match the source` … `vulkan-shaders` | The dev build picked up the release shader cache. `git pull`, then delete `Z:\Projects\Whisper\b\debug\build\whisper-rs-sys-*` and run `npm run tauri dev` again. |
| `file watcher error: UNKNOWN` | Stop that window with Ctrl+C, `git pull`, then `npm run tauri dev` again. Vite polls the network folder, so those lines stay out of the log. The build has stopped only when the window returns to the prompt with `build failed`. |
| `Rename-Item` says Access is denied on `Z:\Projects\Whisper` | A program still has that folder open. Publishing works from `Z:\Projects\Whisper\seda` with the folder under its current name. |

## Privacy

- Audio, transcripts, notes and settings never leave your PC. The only network traffic is the one-time model download and a check on GitHub for new versions every few hours.
- Call detection reads Windows' own "which app is using the microphone" list; Seda records only after you press Start (or if you choose "Record automatically" in Settings).
- Meeting audio is transcribed as you talk and never saved: only the transcript and notes stay (you can turn on "Keep meeting recordings" in Settings).
- Delete any dictation or meeting from the app; any saved audio is deleted with it, and the text is overwritten in the database.

## Security

- **The writing AI is locked down.** It listens only on this PC, on a random port, with a password that is new on every launch and known only to Seda. Its status endpoints and browser access are off, so websites can't reach it, and Seda checks that the process answering is the one it started.
- **Downloads are verified.** Every model and the AI engine are pinned to an exact version and checked against their published SHA-256 fingerprints; anything that doesn't match is deleted and refused.
- **Signed updates.** Seda updates itself from this repository's releases, and only installs an update whose digital signature matches Seda's key; a tampered or fake update is refused.
- **No admin rights, no open ports to the network, no telemetry.**
- **The app's windows are sandboxed** with a strict Content Security Policy, and commands that type into your apps or use the microphone only work from the window they belong to. Model and engine paths are accepted only from Seda's own models folder.
- **Dictated text stays out of Windows clipboard history and cloud clipboard**, is pasted only into the window you started dictating in (never as multiple lines into a terminal), and is never written to the log.
- Found a problem? Please open an issue.

## License

[MIT](LICENSE): free to use, change and share, including commercially. The AI models Seda downloads have their own licenses: Whisper (MIT), the Persian Whisper fine-tune, Silero VAD (MIT), Gemma (Gemma Terms of Use).
