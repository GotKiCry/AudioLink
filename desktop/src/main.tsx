import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// 挂载点缺失时直接抛错：`as HTMLElement` 会把 null 断言掉，换来的是运行时
// 在 createRoot 里报一句难懂的错 —— 宁可在这里就说清楚是 index.html 的问题。
const rootElement = document.getElementById("root");
if (rootElement === null) {
  throw new Error("Root element #root not found; index.html was changed?");
}

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
