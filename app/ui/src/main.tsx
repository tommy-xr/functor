import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

// The macOS window, whose title bar this page draws (see `.native-titlebar`).
if ((window as { functorTitlebar?: boolean }).functorTitlebar) document.documentElement.classList.add("native-titlebar");

createRoot(document.getElementById("root")!).render(<App />);
