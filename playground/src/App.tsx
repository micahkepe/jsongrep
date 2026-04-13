import { Playground } from "./Playground";
import "./index.css";

export function App() {
  return (
    <div className="app">
      <h1>jsongrep</h1>
      <p className="subtitle">
        interactive playground &mdash;{" "}
        <a href="https://github.com/micahkepe/jsongrep">source</a>
      </p>
      <Playground />
    </div>
  );
}
