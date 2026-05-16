/**
 * 屏 5 - 我的 / 设置.
 * 当前只放登出 + 数据清理入口, 后续加 CSR 校准 / 导出 / 关于.
 */
import { useEffect, useState } from "react";
import {
  clearAllCoalPrefs,
  clearHistory,
  clearUserContract,
  getCoalPrefs,
  getHistory,
  getUserContract,
  logout,
} from "../storage";

function getStats() {
  return {
    prefs_count: Object.keys(getCoalPrefs()).length,
    history_count: getHistory().length,
    has_user_contract: getUserContract() != null,
  };
}

interface MeProps {
  onNavigate: (id: "contract" | "history") => void;
}

export function MeScreen({ onNavigate }: MeProps) {
  const [stats, setStats] = useState(getStats());

  useEffect(() => {
    const refresh = () => setStats(getStats());
    window.addEventListener("doudou:prefs_changed", refresh);
    window.addEventListener("doudou:contract_changed", refresh);
    window.addEventListener("doudou:history_changed", refresh);
    return () => {
      window.removeEventListener("doudou:prefs_changed", refresh);
      window.removeEventListener("doudou:contract_changed", refresh);
      window.removeEventListener("doudou:history_changed", refresh);
    };
  }, []);

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

      {/* 配置入口 */}
      <div className="card">
        <div className="card-title">配置 / 数据</div>
        <NavButton
          label="合同指标"
          hint={
            stats.has_user_contract
              ? "已修改 (覆盖 master 默认)"
              : "用 master 默认 8 项"
          }
          onClick={() => onNavigate("contract")}
        />
        <NavButton
          label="历史方案"
          hint={`${stats.history_count} 条`}
          onClick={() => onNavigate("history")}
        />
        <Row label="煤偏好覆盖" value={`${stats.prefs_count} 项`} isLast />
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
          label="清空历史"
          hint="删除所有保存的配煤方案"
          onClick={() => {
            if (confirm("清空所有历史方案?")) clearHistory();
          }}
        />
      </div>

      {/* 关于 */}
      <div className="card">
        <div className="card-title">关于</div>
        <Row label="版本" value="v0.1.1" />
        <Row label="运行模式" value={detectMode()} />
        <Row label="数据源" value="73 煤 master v2.0" isLast />
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

function NavButton({
  label,
  hint,
  onClick,
}: {
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: "flex",
        width: "100%",
        textAlign: "left",
        padding: "12px 0",
        borderBottom: "1px solid var(--c-border)",
        justifyContent: "space-between",
        alignItems: "center",
      }}
    >
      <div>
        <div style={{ fontSize: 14, fontWeight: 500 }}>{label}</div>
        <div style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 2 }}>
          {hint}
        </div>
      </div>
      <span style={{ color: "var(--c-text-3)", fontSize: 18 }}>›</span>
    </button>
  );
}

function DangerButton({
  label,
  hint,
  onClick,
}: {
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: "block",
        width: "100%",
        textAlign: "left",
        padding: "12px 0",
        borderBottom: "1px solid var(--c-border)",
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
