import "./styles.css";
import { mountApp } from "./App";

const root = document.querySelector<HTMLElement>("#app");

if (!root) {
  throw new Error("ChaosLauncher mount point #app is missing from index.html");
}

mountApp(root);
