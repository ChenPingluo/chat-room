import "../styles.css";
import { mount } from "svelte";
import { initTheme } from "../lib/theme.js";
import Admin from "./Admin.svelte";

initTheme();
mount(Admin, { target: document.getElementById("app") });
