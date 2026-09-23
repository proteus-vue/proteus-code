import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// 默认浅色（对齐三家主界面）；localStorage 可覆盖
const saved = localStorage.getItem("neo-theme");
if (saved === "dark" || saved === "light") {
  document.documentElement.setAttribute("data-theme", saved);
} else {
  document.documentElement.setAttribute("data-theme", "light");
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
