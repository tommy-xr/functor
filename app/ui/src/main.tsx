import { createRoot } from "react-dom/client";
import { App } from "./App";
import { poppedPane } from "./layout-model";
import { PopOutPage } from "./PopOutPage";
import "./styles.css";

// A pane in a window of its own, or the main window.
const pane = poppedPane(location.search);

// The macOS main window, whose title bar this page draws (see `.native-titlebar`).
if (pane === null && (window as { functorTitlebar?: boolean }).functorTitlebar) document.documentElement.classList.add("native-titlebar");

createRoot(document.getElementById("root")!).render(pane === null ? <App /> : <PopOutPage pane={pane} />);
