/**
 * 后台卡片 2 - CSR 回归模型.
 *
 * 只看进度: 回填样本攒够之前回归不可信, 也还没接进求解.
 */
import { useEffect, useState } from "react";
import { getCsrModel, type CsrModelStatus } from "../../admin";
import { errorMessage, errorText, hintText } from "./styles";

export function CsrModelCard() {
  const [model, setModel] = useState<CsrModelStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getCsrModel()
      .then((result) => alive && setModel(result))
      .catch((e: unknown) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  return (
    <div className="card">
      <div className="card-title">CSR 回归模型</div>
      <p style={hintText}>样本够了之后再接入求解, 目前不影响配方.</p>
      {error && <div style={errorText}>{error}</div>}
      {!model && !error && (
        <div style={{ fontSize: 12, color: "var(--c-text-3)" }}>加载中...</div>
      )}
      {model && (
        <div style={{ fontSize: 13, lineHeight: 1.8 }}>
          <div>
            已回填 {model.samples} / 需要 {model.required} 条
          </div>
          <div>
            状态:{" "}
            <span
              style={{
                fontWeight: 600,
                color: model.ready ? "var(--c-success, inherit)" : "var(--c-text-3)",
              }}
            >
              {model.ready ? "样本已够" : "样本不足"}
            </span>
          </div>
          {model.r_squared != null && <div>R² = {model.r_squared.toFixed(3)}</div>}
        </div>
      )}
    </div>
  );
}
