import "../styles.css";
import { mount } from "svelte";
import { initTheme } from "../lib/theme.js";
import Admin from "./Admin.svelte";

initTheme();
const root = document.getElementById("app");
mount(Admin, { target: root });
document.getElementById("entry")?.remove();
document.documentElement.dataset.ready = "1";
window.dispatchEvent(new Event("chat-room-ready"));
