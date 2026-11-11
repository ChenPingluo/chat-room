import "../styles.css";
import { mount } from "svelte";
import { initTheme } from "../lib/theme.js";
import App from "./App.svelte";

initTheme();
mount(App, { target: document.getElementById("app") });

if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/sw.js").catch(() => {});
}
