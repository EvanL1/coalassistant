import { useEffect, useState } from "react";
import { getBackend } from "./backend";

const SAMPLE_INPUT = JSON.stringify(
  {
    coals: [
      {
        name: "临北",
        props: { S: 2.0, A: 6.0, V: 22, G: 93, Y: 17, petro: 0.01, CSR: 70, M: 11 },
        fob: 1425,
        frt: 25,
      },
      {
        name: "筛精",
        props: { S: 3.9, A: 9.5, V: 25, G: 100, Y: 22, petro: 0.08, CSR: 65, M: 9.5 },
        fob: 970,
        frt: 30,
      },
    ],
    specs: [
      { indicator: "S", direction: "Upper", max: 3.0, enabled: true },
      { indicator: "V", direction: "Range", min: 18, max: 27, enabled: true },
    ],
    total_quantity: 1000,
    truncate_decimal: true,
  },
  null,
  2
);

function App() {
  const [backendKind, setBackendKind] = useState<string>("loading...");
  const [version, setVersion] = useState<string>("");
  const [solveResult, setSolveResult] = useState<string>("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    getBackend()
      .then(async (b) => {
        setBackendKind(b.kind);
        const v = await b.getVersion();
        setVersion(v);
      })
      .catch((e) => setBackendKind(`error: ${e}`));
  }, []);

  async function runSolve() {
    setLoading(true);
    setSolveResult("");
    try {
      const t0 = performance.now();
      const backend = await getBackend();
      const raw = await backend.solveJson(SAMPLE_INPUT);
      const elapsed = performance.now() - t0;
      const parsed = JSON.parse(raw);
      setSolveResult(
        `耗时: ${elapsed.toFixed(2)}ms\n\n` + JSON.stringify(parsed, null, 2)
      );
    } catch (e) {
      setSolveResult(`Error: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  return (
    <main
      style={{
        fontFamily: "system-ui, sans-serif",
        padding: "1.5rem",
        maxWidth: "720px",
        margin: "0 auto",
      }}
    >
      <h2 style={{ margin: "0 0 1rem 0" }}>豆哥配煤 - 双后端验证</h2>

      <div
        style={{
          background: "#f5f6f8",
          padding: "0.75rem 1rem",
          borderRadius: "8px",
          marginBottom: "1.5rem",
          fontSize: "13px",
        }}
      >
        <div>
          运行模式: <strong>{backendKind}</strong>
          {backendKind === "tauri" && " (原生 IPC)"}
          {backendKind === "wasm" && " (浏览器 WASM)"}
        </div>
        {version && <div>blend_kit 版本: {version}</div>}
      </div>

      <button
        onClick={runSolve}
        disabled={loading || backendKind === "loading..."}
        style={{
          padding: "0.75rem 1.5rem",
          fontSize: "16px",
          background: "#0a5fff",
          color: "white",
          border: "none",
          borderRadius: "8px",
          cursor: "pointer",
        }}
      >
        {loading ? "求解中..." : "跑一次配煤求解"}
      </button>

      {solveResult && (
        <pre
          style={{
            background: "#1e1e1e",
            color: "#e8e8e8",
            padding: "1rem",
            marginTop: "1rem",
            borderRadius: "8px",
            maxHeight: "60vh",
            overflow: "auto",
            fontSize: "12px",
          }}
        >
          {solveResult}
        </pre>
      )}
    </main>
  );
}

export default App;
