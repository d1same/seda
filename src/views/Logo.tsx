import iconSvg from "../assets/seda-icon.svg?raw";

// The hand-lettered "Seda" from the logo pack (src/assets/seda-icon.svg), drawn here so the dot
// can double as a status light.
const LETTERS = iconSvg.match(/<path d="([^"]+)"/)?.[1] ?? "";
const BRAND_DOT = "#ff1f26";

/** The Seda icon: white lettering on a near-black rounded square, with the red dot. `dot` recolours
 *  the dot to show a state (green ready, amber busy, red recording). */
export default function Logo({ size = 20, dot = BRAND_DOT, pulse = false, outline = true }: { size?: number; dot?: string; pulse?: boolean; outline?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden="true" style={{ flex: "none", display: "block" }}>
      <rect width="1024" height="1024" rx="229.9" fill="#121212" stroke={outline ? "rgba(255,255,255,.16)" : "none"} strokeWidth="24" />
      <g transform="translate(0,1024) scale(0.05,-0.05)" fill="#fff">
        <path d={LETTERS} />
      </g>
      <circle cx="831.79" cy="822.54" r="64.55" fill={dot} className={pulse ? "logo-pulse" : undefined} />
    </svg>
  );
}
