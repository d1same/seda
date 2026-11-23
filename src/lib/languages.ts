/** Spoken language choices. "auto" lets Whisper large-v3-turbo detect any of 99+ languages
 *  automatically; Persian ("fa") uses its own fine-tuned model; the rest share multilingual. */
export const LANGUAGES: { code: string; name: string; native: string; rtl?: boolean }[] = [
  { code: "auto", name: "Any language (99+)", native: "Auto-detect" },
  { code: "en", name: "English only", native: "English" },
  { code: "fa", name: "Persian", native: "فارسی", rtl: true },
  { code: "zh", name: "Chinese", native: "中文" },
  { code: "hi", name: "Hindi", native: "हिन्दी" },
  { code: "es", name: "Spanish", native: "Español" },
  { code: "ar", name: "Arabic", native: "العربية", rtl: true },
  { code: "fr", name: "French", native: "Français" },
  { code: "pt", name: "Portuguese", native: "Português" },
  { code: "ru", name: "Russian", native: "Русский" },
  { code: "de", name: "German", native: "Deutsch" },
  { code: "ja", name: "Japanese", native: "日本語" },
  { code: "el", name: "Greek", native: "Ελληνικά" },
];

export const languageName = (code: string) => LANGUAGES.find((l) => l.code === code)?.name ?? code;
