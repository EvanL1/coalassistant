/**
 * 后台 - 管理员账号登录后看到的唯一页面 (不进五屏应用).
 *
 * 四张卡片各自加载各自的数据, 一张失败不拖累其他.
 */
import { logout } from "../auth";
import { ApiKeyCard } from "./ApiKeyCard";
import { CoalOverridesCard } from "./admin/CoalOverridesCard";
import { CsrModelCard } from "./admin/CsrModelCard";
import { RankInteractionCard } from "./admin/RankInteractionCard";

export function AdminScreen() {
  return (
    // 不复用 .app: 宽屏下它是带侧边栏的两列网格, 后台没有 TabBar.
    <div
      style={{
        maxWidth: 760,
        margin: "0 auto",
        minHeight: "100vh",
        padding: 16,
        background: "var(--c-bg)",
      }}
    >
      <div className="page-header" style={{ alignItems: "center" }}>
        <h1 className="page-title">豆哥配煤 · 后台</h1>
        <button
          className="btn btn-secondary"
          style={{ height: 36, padding: "0 16px", fontSize: 13, color: "var(--c-danger)" }}
          onClick={() => {
            if (confirm("退出后台登录?")) void logout();
          }}
        >
          退出登录
        </button>
      </div>

      <RankInteractionCard />
      <CsrModelCard />
      <CoalOverridesCard />
      <ApiKeyCard />
    </div>
  );
}
