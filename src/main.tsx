import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/vazirmatn/400.css";
import "@fontsource/vazirmatn/500.css";
import "@fontsource/vazirmatn/600.css";
import "./styles.css";
import Main from "./views/Main";
import Pill from "./views/Pill";
import Fix from "./views/Fix";
import Call from "./views/Call";

const view = new URLSearchParams(location.search).get("view");
document.documentElement.classList.add(`view-${view ?? "main"}`);
// Before the first paint: a dark color-scheme makes the icon's clear corners white.
if (view === "pill") document.documentElement.style.setProperty("color-scheme", "only light", "important");
const Root = view === "pill" ? Pill : view === "fix" ? Fix : view === "call" ? Call : Main;

// No browser context menu or reload shortcuts in a desktop app.
window.addEventListener("contextmenu", (e) => {
  if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
