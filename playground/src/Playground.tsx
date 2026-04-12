import { useState, type FormEvent } from "react";
import { useId } from "react";
import { jsongrep } from "generated/jsongrep_wasm";

export function Playground() {
  const [data, setData] = useState("");
  const [query, setQuery] = useState("");
  const [output, setOutput] = useState("");
  const [compileTiming, setCompileTiming] = useState("0");
  const [queryTiming, setQueryTiming] = useState("0");

  const queryId = useId();
  const dataId = useId();
  const outputId = useId();

  const handleSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();

    if (!data || !query) {
      setOutput("Please provide both data (JSON/YAML) and a query.");
      return;
    }

    try {
      const beforeRoundtrip = performance.now();
      const resultsWithTimings: jsongrep.TimingResults =
        jsongrep.queryWithTimings(data, query);
      const afterRoundtrip = performance.now();
      const roundtrip = afterRoundtrip - beforeRoundtrip;

      if (localStorage.getItem("JSONGREP_TIMINGS")) {
        console.log({ timings: resultsWithTimings.timings, roundtrip });
      }
      const results: string[] = resultsWithTimings.results.map(
        ([, value]) => value,
      );
      setOutput(
        results.length > 0
          ? results.join("\n\n---\n\n")
          : "No results found matching the query.",
      );
      resultsWithTimings.timings.compileNs === 0n
        ? setCompileTiming("< 1")
        : setCompileTiming(
            `${Number(resultsWithTimings.timings.compileNs) / 1e6}`,
          );
      resultsWithTimings.timings.queryNs === 0n
        ? setQueryTiming("< 1")
        : setQueryTiming(`${Number(resultsWithTimings.timings.queryNs) / 1e6}`);
    } catch (error) {
      let message = "An unknown error occurred.";
      if (typeof error === "string") message = error;
      else if (error instanceof Error) message = error.message;
      setOutput(`ERROR: ${message}`);
    }
  };

  return (
    <div className="api-tester">
      <div className="inputs-panel">
        <div className="input-group">
          <label htmlFor={dataId}>Data (JSON / YAML)</label>
          <textarea
            id={dataId}
            value={data}
            onChange={(e) => setData(e.target.value)}
            placeholder="Paste your JSON or YAML data here."
            className="textarea data-box"
          />
        </div>

        <div className="input-group">
          <label htmlFor={queryId}>Query</label>
          <textarea
            id={queryId}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="e.g. users[*].name"
            className="textarea query-box"
          />
        </div>

        <form onSubmit={handleSubmit}>
          <button type="submit" className="run-button">
            Run Query
          </button>
        </form>
      </div>

      <div className="output-panel">
        <label htmlFor={outputId}>Results</label>
        <textarea
          id={outputId}
          value={output}
          readOnly
          placeholder="Results will appear here…"
          className="textarea output-box"
        />
      </div>
      <div>
        compile query: {compileTiming} ms and run query: {queryTiming} ms
      </div>
    </div>
  );
}
