/** English is always understood. This is the one extra language you can add ("en" = none, English
 *  only). Persian uses its own fine-tuned model; the rest share the multilingual one. */
export const LANGUAGES: { code: string; name: string; native: string; rtl?: boolean }[] = [
  { code: "en", name: "English only", native: "None" },
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
