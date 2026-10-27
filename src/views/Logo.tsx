import iconSvg from "../assets/seda-icon.svg?raw";

// The hand-lettered "Seda" from the logo pack (src/assets/seda-icon.svg), drawn here so the dot
// can double as a status light.
const LETTERS = iconSvg.match(/<path d="([^"]+)"/)?.[1] ?? "";
const BRAND_DOT = "#ff1f26";

/** The Seda icon: white lettering on a near-black rounded square, with the red dot. `dot` recolours
 *  the dot to show a state. `plateOpacity` below 1 lets the desktop show through the tile.
 *  `radius` is the corner curve on the 1024 canvas. */
export default function Logo({ size = 20, dot = BRAND_DOT, pulse = false, outline = true, plate = true, fill = false, plateOpacity = 1, radius = 229.9 }: { size?: number; dot?: string; pulse?: boolean; outline?: boolean; plate?: boolean; fill?: boolean; plateOpacity?: number; radius?: number }) {
  return (
    <svg width={fill ? "100%" : size} height={fill ? "100%" : size} viewBox="0 0 1024 1024" aria-hidden="true" style={fill ? { display: "block", width: "100%", height: "100%" } : { flex: "none", display: "block" }}>
      {plate && <rect width="1024" height="1024" rx={radius} fill="#121212" fillOpacity={plateOpacity} stroke={outline ? "rgba(255,255,255,.16)" : "none"} strokeWidth="24" />}
      <g transform="translate(0,1024) scale(0.05,-0.05)" fill="#fff">
        <path d={LETTERS} />
      </g>
      <circle cx="831.79" cy="822.54" r="64.55" fill={dot} className={pulse ? "logo-pulse" : undefined} />
    </svg>
  );
}
