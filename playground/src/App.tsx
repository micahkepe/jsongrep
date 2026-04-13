import { useCallback, useEffect, useState } from "react";
import { Playground } from "./Playground";
import "./index.css";

type Theme = "light" | "dark";

function getInitialTheme(): Theme {
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

export function App() {
  const [theme, setTheme] = useState<Theme>(getInitialTheme);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("theme", theme);
  }, [theme]);

  const toggleTheme = useCallback(() => {
    setTheme((t) => (t === "dark" ? "light" : "dark"));
  }, []);

  return (
    <div className="app">
      <header>
        <hgroup>
          <h1>jsongrep</h1>
          <p className="subtitle">
            interactive playground &mdash;{" "}
            <a href="https://github.com/micahkepe/jsongrep">source</a>
          </p>
        </hgroup>
        <button
          type="button"
          className="theme-toggle"
          onClick={toggleTheme}
          aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
        >
          {theme === "dark" ? "light" : "dark"}
        </button>
      </header>
      <Playground />
    </div>
  );
}
