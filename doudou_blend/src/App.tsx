import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const SAMPLE_INPUT = JSON.stringify({
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
}, null, 2);

function App() {
  const [versionResult, setVersionResult] = useState<string>("");
  const [solveResult, setSolveResult] = useState<string>("");
  const [loading, setLoading] = useState<{ version: boolean; solve: boolean }>({
    version: false,
    solve: false,
  });

  async function checkVersion() {
    setLoading((l) => ({ ...l, version: true }));
    try {
      const v = await invoke<string>("version");
      setVersionResult(v);
    } catch (e) {
      setVersionResult(`Error: ${e}`);
    } finally {
      setLoading((l) => ({ ...l, version: false }));
    }
  }

  async function runSolve() {
    setLoading((l) => ({ ...l, solve: true }));
    try {
      const raw = await invoke<string>("solve_blend", { inputJson: SAMPLE_INPUT });
      setSolveResult(JSON.stringify(JSON.parse(raw), null, 2));
    } catch (e) {
      setSolveResult(`Error: ${e}`);
    } finally {
      setLoading((l) => ({ ...l, solve: false }));
    }
  }

  return (
    <main style={{ fontFamily: "monospace", padding: "1rem" }}>
      <h2>配煤求解验证 Demo</h2>

      <section style={{ marginBottom: "1.5rem" }}>
        <button onClick={checkVersion} disabled={loading.version}>
          {loading.version ? "..." : "检查 Rust 版本"}
        </button>
        {versionResult && (
          <pre style={{ background: "#f0f0f0", padding: "0.5rem", marginTop: "0.5rem" }}>
            {versionResult}
          </pre>
        )}
      </section>

      <section>
        <button onClick={runSolve} disabled={loading.solve}>
          {loading.solve ? "求解中..." : "跑一次配煤求解"}
        </button>
        {solveResult && (
          <pre
            style={{
              background: "#f0f0f0",
              padding: "0.5rem",
              marginTop: "0.5rem",
              maxHeight: "60vh",
              overflow: "auto",
            }}
          >
            {solveResult}
          </pre>
        )}
      </section>
    </main>
  );
}

export default App;
