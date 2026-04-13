import { useState, useId, useRef, type FormEvent, type KeyboardEvent } from "react";
import { jsongrep } from "generated/jsongrep_wasm";

const MAX_INPUT_SIZE = 1_000_000;

const DEFAULT_DATA = `{
  "users": [
    { "name": "Alice", "age": 30, "role": "admin" },
    { "name": "Bob", "age": 25, "role": "user" },
    { "name": "Charlie", "age": 35, "role": "user" }
  ]
}`;

const DEFAULT_QUERY = "users[*].name";

export function Playground() {
  const [data, setData] = useState(DEFAULT_DATA);
  const [query, setQuery] = useState(DEFAULT_QUERY);
  const [results, setResults] = useState<[string, string][]>([]);
  const [error, setError] = useState("");
  const [compileTiming, setCompileTiming] = useState("0");
  const [queryTiming, setQueryTiming] = useState("0");

  const queryId = useId();
  const dataId = useId();
  const formRef = useRef<HTMLFormElement>(null);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      formRef.current?.requestSubmit();
    }
  };

  const handleSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();

    if (!data || !query) {
      setError("Please provide both data (JSON/YAML) and a query.");
      setResults([]);
      return;
    }

    if (data.length > MAX_INPUT_SIZE) {
      setError(
        `Input too large (${(data.length / 1_000_000).toFixed(1)} MB, max 1 MB).`,
      );
      setResults([]);
      return;
    }

    try {
      setError("");
      const beforeRoundtrip = performance.now();
      const resultsWithTimings: jsongrep.TimingResults =
        jsongrep.queryWithTimings(data, query);
      const afterRoundtrip = performance.now();
      const roundtrip = afterRoundtrip - beforeRoundtrip;

      if (localStorage.getItem("JSONGREP_TIMINGS")) {
        console.log({ timings: resultsWithTimings.timings, roundtrip });
      }
      setResults(resultsWithTimings.results);

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
      setResults([]);
    }
  };

  return (
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
          onKeyDown={handleKeyDown}
          placeholder="Paste your JSON or YAML data here."
          className="textarea data-box"
          spellCheck={false}
        />

        <label htmlFor={queryId}>Query</label>
        <textarea
          id={queryId}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="e.g. users[*].name"
          className="textarea query-box"
          spellCheck={false}
        />

        <footer className="button-row">
          <button type="submit" className="run-button" aria-label="Run query">
            Run Query
          </button>
          <kbd aria-label="Keyboard shortcut: Command or Control plus Enter">
            Cmd+Enter
          </kbd>
        </footer>
      </fieldset>

      <fieldset className="output-panel" aria-live="polite">
        <legend>Results</legend>
        <output className="output-box">
          {error ? (
            <samp className="output-error">{error}</samp>
          ) : results.length > 0 ? (
            <dl className="result-list">
              {results.map(([path, value], i) => (
                <div className="result-entry" key={i}>
                  <dt>{path}:</dt>
                  <dd><pre>{value}</pre></dd>
                </div>
              ))}
            </dl>
          ) : (
            <p className="output-placeholder">Results will appear here...</p>
          )}
        </output>
      </fieldset>

      <footer className="timing-bar" aria-label="Query performance timings">
        compile: <data value={compileTiming}>{compileTiming} ms</data>
        {" / "}
        query: <data value={queryTiming}>{queryTiming} ms</data>
      </footer>
    </form>
  );
}
