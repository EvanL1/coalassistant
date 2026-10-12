/**
 * 管理端 - API 密钥.
 *
 * 生成 / 命名 / 销毁两种用途的密钥: data 给协作者写煤库指标, mcp 给手机 Agent
 * (Kimi 等) 经 /mcp 只读 + 求解。
 * 服务端只存哈希, 明文仅在创建响应里出现一次, 所以这里必须当场让用户复制,
 * 并把"只显示这一次"讲清楚 —— 关掉就再也拿不回来了.
 */
import { useEffect, useState } from "react";
import {
  createApiKey,
  listApiKeys,
  revokeApiKey,
  type ApiKey,
  type KeyScope,
} from "../apiKeys";

const SCOPE_LABEL: Record<KeyScope, string> = {
  data: "写煤库",
  mcp: "手机 Agent",
};

export function ApiKeyCard() {
  const [keys, setKeys] = useState<ApiKey[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [scope, setScope] = useState<KeyScope>("mcp");
  const [creating, setCreating] = useState(false);
  /** 刚生成的明文, 只在本次会话内存活, 不写任何存储. */
  const [freshKey, setFreshKey] = useState<{
    name: string;
    scope: KeyScope;
    plaintext: string;
  } | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let alive = true;
    listApiKeys()
      .then((rows) => alive && setKeys(rows))
      .catch((e: unknown) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
    };
  }, []);

  async function refresh() {
    try {
      setKeys(await listApiKeys());
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handleCreate() {
    const trimmed = name.trim();
    if (!trimmed || creating) return;
    setCreating(true);
    setError(null);
    try {
      const { plaintext } = await createApiKey(trimmed, scope);
      setFreshKey({ name: trimmed, scope, plaintext });
      setCopied(false);
      setName("");
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }

  async function handleRevoke(key: ApiKey) {
    if (!confirm(`销毁「${key.name}」? 持有者将立即无法写入, 且不可恢复.`)) return;
    setError(null);
    try {
      await revokeApiKey(key.id);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const active = keys?.filter((k) => !k.revoked_at) ?? [];
  const revoked = keys?.filter((k) => k.revoked_at) ?? [];

  return (
    <div className="card">
      <div className="card-title">API 密钥</div>
      <p style={{ fontSize: 11, color: "var(--c-text-3)", margin: "0 0 12px" }}>
        「手机 Agent」给同事在 Kimi 等 App 里调配煤 (只读 + 求解, 改不了数据)；「写煤库」给协作者补煤库指标。一人一把, 销毁后立即失效, 不影响其他人。
      </p>

      {/* 新生成的明文: 只显示这一次 */}
      {freshKey && (
        <div
          style={{
            border: "1px solid var(--c-warning, #d08700)",
            borderRadius: 8,
            padding: 12,
            marginBottom: 12,
          }}
        >
          <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
            「{freshKey.name}」的密钥 · 只显示这一次
          </div>
          <code
            style={{
              display: "block",
              fontSize: 11,
              wordBreak: "break-all",
              background: "var(--c-bg-2, rgba(0,0,0,.05))",
              padding: 8,
              borderRadius: 6,
              marginBottom: 8,
            }}
          >
            {freshKey.plaintext}
          </code>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              className="btn btn-secondary"
              style={{ height: 32, padding: "0 12px", fontSize: 12 }}
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(freshKey.plaintext)
                  .then(() => setCopied(true))
                  .catch(() => setError("复制失败, 请手动选中复制"));
              }}
            >
              {copied ? "已复制" : "复制"}
            </button>
            <button
              className="btn btn-secondary"
              style={{ height: 32, padding: "0 12px", fontSize: 12 }}
              onClick={() => setFreshKey(null)}
            >
              我已保存
            </button>
          </div>
          {freshKey.scope === "mcp" && (
            <div style={{ fontSize: 11, marginTop: 8, lineHeight: 1.6 }}>
              在 Kimi 等 App 添加 MCP 服务, 二选一:
              <br />· 地址 <code>{window.location.origin}/mcp</code>, 请求头{" "}
              <code>Authorization: Bearer 上面的密钥</code>
              <br />· 只能填一个地址时用{" "}
              <code style={{ wordBreak: "break-all" }}>
                {window.location.origin}/mcp/{freshKey.plaintext}
              </code>
              (这个地址本身就是密钥, 别外传)
            </div>
          )}
          <div style={{ fontSize: 10, color: "var(--c-text-3)", marginTop: 6 }}>
            服务端只存哈希，关掉后无法再取回。丢了就销毁重建。
          </div>
        </div>
      )}

      {/* 新建 */}
      <div style={{ display: "flex", gap: 8, marginBottom: 12, flexWrap: "wrap" }}>
        <select
          aria-label="密钥用途"
          value={scope}
          onChange={(e) => setScope(e.target.value as KeyScope)}
          style={{
            height: 36,
            padding: "0 8px",
            fontSize: 13,
            borderRadius: 8,
            border: "1px solid var(--c-border, rgba(0,0,0,.15))",
            background: "transparent",
            color: "inherit",
          }}
        >
          <option value="mcp">手机 Agent (只读+求解)</option>
          <option value="data">写煤库</option>
        </select>
        <input
          value={name}
          maxLength={64}
          placeholder="名字, 如: 李四-手机"
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void handleCreate();
          }}
          style={{
            flex: 1,
            minWidth: 140,
            height: 36,
            padding: "0 10px",
            fontSize: 13,
            borderRadius: 8,
            border: "1px solid var(--c-border, rgba(0,0,0,.15))",
            background: "transparent",
            color: "inherit",
          }}
        />
        <button
          className="btn btn-secondary"
          style={{ height: 36, padding: "0 16px", fontSize: 13 }}
          disabled={!name.trim() || creating}
          onClick={() => void handleCreate()}
        >
          {creating ? "生成中..." : "生成"}
        </button>
      </div>

      {error && (
        <div style={{ fontSize: 11, color: "var(--c-danger)", marginBottom: 8 }}>
          {error}
        </div>
      )}

      {keys == null && !error && (
        <div style={{ fontSize: 12, color: "var(--c-text-3)" }}>加载中...</div>
      )}

      {keys != null && active.length === 0 && (
        <div style={{ fontSize: 12, color: "var(--c-text-3)" }}>
          还没有密钥。生成一把发给协作者即可。
        </div>
      )}

      {active.map((key) => (
        <KeyRow key={key.id} item={key} onRevoke={() => void handleRevoke(key)} />
      ))}

      {revoked.length > 0 && (
        <details style={{ marginTop: 10 }}>
          <summary style={{ fontSize: 11, color: "var(--c-text-3)", cursor: "pointer" }}>
            已销毁 {revoked.length} 把
          </summary>
          {revoked.map((key) => (
            <KeyRow key={key.id} item={key} />
          ))}
        </details>
      )}
    </div>
  );
}

function KeyRow({ item, onRevoke }: { item: ApiKey; onRevoke?: () => void }) {
  const revoked = item.revoked_at != null;
  return (
    <div
      style={{
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        gap: 8,
        padding: "8px 0",
        borderTop: "1px solid var(--c-border, rgba(0,0,0,.08))",
        opacity: revoked ? 0.5 : 1,
      }}
    >
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 500 }}>
          {item.name}
          <span style={{ fontSize: 10, color: "var(--c-text-3)", marginLeft: 6 }}>
            {SCOPE_LABEL[item.scope ?? "data"]}
          </span>
          {revoked && (
            <span style={{ fontSize: 10, color: "var(--c-text-3)", marginLeft: 6 }}>
              已销毁
            </span>
          )}
        </div>
        <div style={{ fontSize: 10, color: "var(--c-text-3)" }}>
          {item.key_prefix}… · {formatUsage(item)}
        </div>
      </div>
      {onRevoke && (
        <button
          className="btn btn-secondary"
          style={{
            height: 30,
            padding: "0 12px",
            fontSize: 12,
            color: "var(--c-danger)",
            flexShrink: 0,
          }}
          onClick={onRevoke}
        >
          销毁
        </button>
      )}
    </div>
  );
}

/** 「最近使用」比「创建时间」更能回答"这把还能不能销毁". */
function formatUsage(item: ApiKey): string {
  if (item.revoked_at) return `销毁于 ${formatDate(item.revoked_at)}`;
  if (!item.last_used_at) return "从未使用";
  return `最近使用 ${formatDate(item.last_used_at)}`;
}

function formatDate(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleDateString("zh-CN");
}
