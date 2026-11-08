import { mount } from "svelte";
import "./lib/design/tokens.css";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });
