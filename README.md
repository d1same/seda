<p align="center"><img src="src-tauri/icons/seda-icon.svg" width="96" alt="Seda"></p>

# Seda

Private voice typing and meeting notes for Windows. Everything runs on your own computer: nothing you say is uploaded.

**[Download the installer](https://github.com/d1same/seda/releases/latest)** (Windows 10/11, 64-bit)

- **Dictation anywhere.** Hold a shortcut (any key or mouse button, e.g. Mouse 4), talk, let go: clean, punctuated text is pasted where your cursor is. Say "no wait…" and Seda drops the part you took back.
- **English + one more language.** Mix them in the same sentence. Persian has its own fine-tuned speech model; Chinese, Hindi, Spanish, Arabic, French, Portuguese, Russian, German, Japanese and Greek use the multilingual one.
- **Writing AI.** A local model adds punctuation and paragraphs, and matches the app (email layout in Outlook/Gmail, bullets in Jira, casual in Teams/Slack).
- **Fix my text.** Select text anywhere, press a shortcut: correct grammar, make it formal, friendly, shorter, or translate.
- **Meetings.** Records you (microphone) and everyone else (the computer's audio), transcribes live, and writes notes with decisions and action items when the call ends. No bot joins your call. Seda notices when Teams, Zoom, Meet and others start a call and offers to take notes.
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

Check the tool paths in `src-tauri/.cargo/config.toml`, then:

```bash
npm install
npm run tauri build
```

The installer is written to `<target-dir>/release/bundle/nsis/`. For development: `npm run tauri dev`. Running `npm run dev` alone opens the interface in a browser with sample data.

## Privacy

- Audio, transcripts, notes and settings never leave your PC. The only network traffic is the one-time model download.
- Call detection reads Windows' own "which app is using the microphone" list; Seda records only after you press Start (or if you choose "Record automatically" in Settings).
- Meeting audio is transcribed as you talk and never saved: only the transcript and notes stay (you can turn on "Keep meeting recordings" in Settings).
- Delete any dictation or meeting from the app; any saved audio is deleted with it.

## License

[MIT](LICENSE): free to use, change and share, including commercially. The AI models Seda downloads have their own licenses: Whisper (MIT), the Persian Whisper fine-tune, Silero VAD (MIT), Gemma (Gemma Terms of Use).
