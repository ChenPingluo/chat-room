import "../styles.css";
import { mount } from "svelte";
import { initTheme } from "../lib/theme.js";
import App from "./App.svelte";

initTheme();
const root = document.getElementById("app");
mount(App, { target: root });
document.getElementById("entry")?.remove();
document.documentElement.dataset.ready = "1";
window.dispatchEvent(new Event("chat-room-ready"));

if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/sw.js", { updateViaCache: "none" }).then((registration) => {
    registration.update().catch(() => {});
  }).catch(() => {});
}

const viewport = window.visualViewport;
if (viewport) {
  const fit = () => {
    const height = Math.round(viewport.height);
    document.documentElement.style.setProperty("--app-height", `${height}px`);
    const typing = document.activeElement?.matches("textarea, input");
    const keyboard = typing && window.innerHeight - viewport.height > 80;
    document.documentElement.classList.toggle("kb-open", keyboard);
    if (keyboard) window.scrollTo(0, 0);
  };
  viewport.addEventListener("resize", fit);
  viewport.addEventListener("scroll", fit);
  fit();
}
