/**
 * 屏 5 - 我的 / 设置.
 * 当前只放登出 + 数据清理入口, 后续加 CSR 校准 / 导出 / 关于.
 */
import { useEffect, useRef, useState } from "react";
import { getBackend } from "../backend";
import {
  clearAllCoalPrefs,
  clearUserContract,
  getCoalPrefs,
  getUserContract,
  logout,
} from "../storage";

interface Stats {
  prefs_count: number;
  history_count: number | null;
  has_user_contract: boolean;
}

function getInitialStats(): Stats {
  return {
    prefs_count: Object.keys(getCoalPrefs()).length,
    history_count: null,
    has_user_contract: getUserContract() != null,
  };
}

export function MeScreen() {
  const [stats, setStats] = useState<Stats>(getInitialStats);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [clearingHistory, setClearingHistory] = useState(false);
  const historyRequestRef = useRef(0);
  const clearingHistoryRef = useRef(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    const refreshLocalStats = () => {
      const current = getInitialStats();
      setStats((previous) => ({
        ...previous,
        prefs_count: current.prefs_count,
        has_user_contract: current.has_user_contract,
      }));
    };
    const refreshHistoryCount = async () => {
      const requestId = ++historyRequestRef.current;
      try {
        const backend = await getBackend();
        const count = await backend.countHistory();
        if (
          !mountedRef.current ||
          requestId !== historyRequestRef.current
        ) {
          return;
        }
        setStats((previous) => ({ ...previous, history_count: count }));
        setHistoryError(null);
      } catch {
        if (
          mountedRef.current &&
          requestId === historyRequestRef.current
        ) {
          setHistoryError("历史数量读取失败");
        }
      }
    };
    const onHistoryChange = () => void refreshHistoryCount();

    void refreshHistoryCount();
    window.addEventListener("doudou:prefs_changed", refreshLocalStats);
    window.addEventListener("doudou:contract_changed", refreshLocalStats);
    window.addEventListener("doudou:history_changed", onHistoryChange);
    return () => {
      mountedRef.current = false;
      historyRequestRef.current += 1;
      window.removeEventListener("doudou:prefs_changed", refreshLocalStats);
      window.removeEventListener("doudou:contract_changed", refreshLocalStats);
      window.removeEventListener("doudou:history_changed", onHistoryChange);
    };
  }, []);

  async function clearBackendHistory() {
    if (
      clearingHistoryRef.current ||
      !confirm("清空所有历史方案?")
    ) {
      return;
    }
    clearingHistoryRef.current = true;
    const requestId = ++historyRequestRef.current;
    setClearingHistory(true);
    setHistoryError(null);
    try {
      const backend = await getBackend();
      await backend.clearHistory();
      // Backend 会同步派发 history_changed；清空完成后重新成为最新代，
      // 确定性写入 0，避免事件触发的旧计数挂起或失败后覆盖结果。
      historyRequestRef.current += 1;
      if (mountedRef.current) {
        setStats((previous) => ({ ...previous, history_count: 0 }));
        setHistoryError(null);
      }
    } catch {
      if (requestId === historyRequestRef.current) {
        historyRequestRef.current += 1;
      }
      if (mountedRef.current) {
        setHistoryError("清空历史失败，请重试");
      }
    } finally {
      clearingHistoryRef.current = false;
      if (mountedRef.current) {
        setClearingHistory(false);
      }
    }
  }

  const historyCountText =
    stats.history_count == null
      ? historyError
        ? "读取失败"
        : "读取中..."
      : `${stats.history_count} 条`;

  return (
    <>
      <div className="page-header">
        <h1 className="page-title">我的</h1>
      </div>

      {/* 账户卡 */}
      <div className="card">
        <div className="card-title">账户</div>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <div>
            <div style={{ fontSize: 15, fontWeight: 600 }}>doudou</div>
            <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
              已登录 · 全部数据本地保存
            </div>
          </div>
          <button
            className="btn btn-secondary"
            style={{
              height: 36,
              padding: "0 16px",
              fontSize: 13,
              color: "var(--c-danger)",
            }}
            onClick={() => {
              if (confirm("退出登录? 本地数据不会被清除.")) logout();
            }}
          >
            退出登录
          </button>
        </div>
      </div>

      {/* 数据状态 */}
      <div className="card">
        <div className="card-title">本地数据</div>
        <Row label="煤偏好覆盖" value={`${stats.prefs_count} 项`} />
        <Row
          label="自定义合同"
          value={stats.has_user_contract ? "已修改" : "用默认"}
        />
        <Row label="历史方案" value={historyCountText} isLast />
        {historyError && (
          <div style={{ fontSize: 11, color: "var(--c-danger)", marginTop: 4 }}>
            {historyError}
          </div>
        )}
      </div>

      {/* 危险区 */}
      <div className="card">
        <div className="card-title" style={{ color: "var(--c-danger)" }}>
          危险操作
        </div>
        <DangerButton
          label="清空煤偏好"
          hint="重置所有煤的启用/价格/化验为 master 默认"
          onClick={() => {
            if (confirm("清空所有煤偏好? 改过的价格/化验值都会丢失.")) {
              clearAllCoalPrefs();
            }
          }}
        />
        <DangerButton
          label="重置合同"
          hint="清除自定义合同, 回到 master 默认 8 项约束"
          onClick={() => {
            if (confirm("重置合同为 master 默认?")) clearUserContract();
          }}
        />
        <DangerButton
          label={clearingHistory ? "清空中..." : "清空历史"}
          hint="删除所有保存的配煤方案"
          disabled={clearingHistory}
          onClick={() => void clearBackendHistory()}
        />
      </div>

      {/* 关于 */}
      <div className="card">
        <div className="card-title">关于</div>
        <Row label="版本" value="v0.1.1" />
        <Row label="运行模式" value={detectMode()} />
        <Row label="数据源" value="内置 Master" isLast />
      </div>

      <p
        style={{
          fontSize: 11,
          color: "var(--c-text-3)",
          textAlign: "center",
          marginTop: 20,
          marginBottom: 12,
        }}
      >
        豆哥配煤 · 全部计算在本地完成 · 数据不上传
      </p>
    </>
  );
}

function detectMode(): string {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    return "Tauri 原生 IPC";
  }
  return "浏览器 WASM";
}

function Row({
  label,
  value,
  isLast,
}: {
  label: string;
  value: string;
  isLast?: boolean;
}) {
  return (
    <div
      style={{
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        padding: "10px 0",
        borderBottom: isLast ? "none" : "1px solid var(--c-border)",
        fontSize: 14,
      }}
    >
      <span style={{ color: "var(--c-text-2)" }}>{label}</span>
      <span style={{ fontWeight: 500 }}>{value}</span>
    </div>
  );
}

function DangerButton({
  label,
  hint,
  onClick,
  disabled = false,
}: {
  label: string;
  hint: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: "block",
        width: "100%",
        textAlign: "left",
        padding: "12px 0",
        borderBottom: "1px solid var(--c-border)",
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <div
        style={{
          fontSize: 14,
          color: "var(--c-danger)",
          fontWeight: 500,
        }}
      >
        {label}
      </div>
      <div style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 2 }}>
        {hint}
      </div>
    </button>
  );
}
