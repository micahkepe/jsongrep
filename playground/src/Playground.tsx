import { useState, useId, useRef, useEffect, type SyntheticEvent } from "react";
import { jsongrep } from "generated/jsongrep_wasm";

const MAX_INPUT_SIZE = 1_000_000;
const IS_MAC =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);
const MOD_KEY = IS_MAC ? "\u2318" : "Ctrl";

const DEFAULT_DATA = `{
  "users": [
    { "name": "Alice", "age": 30, "role": "admin" },
    { "name": "Bob", "age": 25, "role": "user" },
    { "name": "Charlie", "age": 35, "role": "user" }
  ]
}`;

const DEFAULT_QUERY = "users[*].name";

const SYNTAX_ROWS: [string, string, string][] = [
  ["Sequence", "foo.bar.baz", "Concatenation: match path foo \u2192 bar \u2192 baz"],
  ["Disjunction", "foo | bar", "Union: match either foo or bar"],
  ["Kleene star", "**", "Match zero or more field accesses"],
  ["Repetition", "foo*", "Repeat the preceding step zero or more times"],
  ["Wildcards", "* or [*]", "Match any single field or array index"],
  ["Optional", "foo?.bar", "Optional foo field access"],
  ["Field access", 'foo or "foo bar"', "Match a specific field (quote if spaces)"],
  ["Array index", "[0] or [1:3]", "Match specific index or slice (exclusive end)"],
  ["Grouping", "foo.(bar|baz).qux", "Parentheses for nesting: matches foo.bar.qux or foo.baz.qux"],
  ["Deep descent", "(* | [*])*.foo", "Recursive descent: find foo at any depth"],
];

export function Playground() {
  const [data, setData] = useState(DEFAULT_DATA);
  const [query, setQuery] = useState(DEFAULT_QUERY);
  const [results, setResults] = useState<[string, string][] | null>(null);
  const [error, setError] = useState("");
  const [compileTiming, setCompileTiming] = useState("0");
  const [queryTiming, setQueryTiming] = useState("0");
  const [runCount, setRunCount] = useState(0);
  const [flash, setFlash] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);

  const queryId = useId();
  const dataId = useId();
  const formRef = useRef<HTMLFormElement>(null);
  const outputRef = useRef<HTMLOutputElement>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    if (runCount === 0) return;
    setFlash(true);
    const id = setTimeout(() => setFlash(false), 150);
    return () => clearTimeout(id);
  }, [runCount]);

  useEffect(() => {
    if (helpOpen) {
      dialogRef.current?.showModal();
    } else {
      dialogRef.current?.close();
    }
  }, [helpOpen]);

  useEffect(() => {
    const handler = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        formRef.current?.requestSubmit();
        return;
      }
      if (e.key === "?" && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const tag = (e.target as HTMLElement)?.tagName;
        if (tag === "TEXTAREA" || tag === "INPUT") return;
        e.preventDefault();
        setHelpOpen((open) => !open);
      }
      if (e.key === "Escape") {
        if (helpOpen) {
          setHelpOpen(false);
        } else if (document.activeElement instanceof HTMLElement) {
          document.activeElement.blur();
        }
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [helpOpen]);

  const handleSubmit = (e: SyntheticEvent<HTMLFormElement, SubmitEvent>) => {
    e.preventDefault();

    if (!data) {
      setError("Please provide data (JSON/YAML).");
      setResults(null);
      return;
    }

    if (data.length > MAX_INPUT_SIZE) {
      setError(
        `Input too large (${(data.length / 1_000_000).toFixed(1)} MB, max 1 MB).`,
      );
      setResults(null);
      return;
    }

    try {
      setError("");
      setRunCount((c) => c + 1);
      const beforeRoundtrip = performance.now();
      const resultsWithTimings: jsongrep.TimingResults =
        jsongrep.queryWithTimings(data, query);
      const afterRoundtrip = performance.now();
      const roundtrip = afterRoundtrip - beforeRoundtrip;

      if (localStorage.getItem("JSONGREP_TIMINGS")) {
        console.log({ timings: resultsWithTimings.timings, roundtrip });
      }
      setResults(resultsWithTimings.results);
      outputRef.current?.scrollTo(0, 0);

      if (resultsWithTimings.timings.compileNs === 0n) {
        setCompileTiming("< 1");
      } else {
        setCompileTiming(
          `${Number(resultsWithTimings.timings.compileNs / 1_000_000n)}`,
        );
      }
      if (resultsWithTimings.timings.queryNs === 0n) {
        setQueryTiming("< 1");
      } else {
        setQueryTiming(
          `${Number(resultsWithTimings.timings.queryNs / 1_000_000n)}`,
        );
      }
    } catch (err) {
      let message = "An unknown error occurred.";
      if (typeof err === "string") message = err;
      else if (err instanceof Error) message = err.message;
      setError(message);
      setResults(null);
    }
  };

  return (
    <>
      <form
        className="api-tester"
        onSubmit={handleSubmit}
        ref={formRef}
        aria-label="JSONGrep query playground"
      >
        <fieldset className="inputs-panel">
          <legend>Input</legend>

          <label htmlFor={dataId}>Data (JSON / YAML)</label>
          <textarea
            id={dataId}
            value={data}
            onChange={(e) => setData(e.target.value)}

            placeholder="Paste your JSON or YAML data here."
            className="textarea data-box"
            spellCheck={false}
          />

          <label htmlFor={queryId}>Query</label>
          <textarea
            id={queryId}
            value={query}
            onChange={(e) => setQuery(e.target.value)}

            placeholder="e.g. users[*].name"
            className="textarea query-box"
            spellCheck={false}
          />

          <footer className="button-row">
            <button type="submit" className="run-button" aria-label="Run query">
              Run Query
            </button>
            <kbd aria-label={`Keyboard shortcut: ${MOD_KEY} plus Enter`}>
              {MOD_KEY} + Enter
            </kbd>
          </footer>
        </fieldset>

        <fieldset className="output-panel" aria-live="polite">
          <legend>Results</legend>
          <output className={`output-box${flash ? " flash" : ""}`} ref={outputRef}>
            {error ? (
              <samp className="output-error">{error}</samp>
            ) : results === null ? (
              <p className="output-placeholder">Results will appear here...</p>
            ) : results.length === 0 ? (
              <p className="output-placeholder">No results found matching the query.</p>
            ) : (
              <dl className="result-list">
                {results.map(([path, value], i) => (
                  <div className="result-entry" key={i}>
                    {path && <dt>{path}:</dt>}
                    <dd><pre>{value}</pre></dd>
                  </div>
                ))}
              </dl>
            )}
          </output>
        </fieldset>

        <footer className="timing-bar" aria-label="Query performance timings">
          <span>
            compile: <data value={compileTiming}>{compileTiming} ms</data>
            {" / "}
            query: <data value={queryTiming}>{queryTiming} ms</data>
          </span>
          <span
            className="help-hint"
            role="button"
            tabIndex={0}
            aria-label="Press ? for query syntax help"
            onClick={() => setHelpOpen(true)}
            onKeyDown={(e) => e.key === "Enter" && setHelpOpen(true)}
          >
            type <kbd>?</kbd> for query syntax
          </span>
        </footer>
      </form>

      <dialog
        ref={dialogRef}
        className="help-dialog"
        aria-label="Query syntax reference"
        onClose={() => setHelpOpen(false)}
      >
        <header className="help-header">
          <h2>Query Syntax</h2>
          <button
            type="button"
            className="help-close"
            aria-label="Close help"
            onClick={() => setHelpOpen(false)}
          >
            Esc
          </button>
        </header>
        <table className="help-table">
          <thead>
            <tr>
              <th scope="col">Operator</th>
              <th scope="col">Example</th>
              <th scope="col">Description</th>
            </tr>
          </thead>
          <tbody>
            {SYNTAX_ROWS.map(([op, example, desc]) => (
              <tr key={op}>
                <td>{op}</td>
                <td><code>{example}</code></td>
                <td>{desc}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </dialog>
    </>
  );
}
