import { useState } from "react";
import "./App.css";
import { TabBar, type TabId } from "./TabBar";
import { TodayScreen } from "./screens/TodayScreen";
import { CoalPoolScreen } from "./screens/CoalPoolScreen";
import { PlaceholderScreen } from "./screens/PlaceholderScreen";

function App() {
  const [tab, setTab] = useState<TabId>("today");

  return (
    <div className="app">
      <div className="app-content">
        {tab === "today" && <TodayScreen />}
        {tab === "pool" && <CoalPoolScreen />}
        {tab === "contract" && (
          <PlaceholderScreen
            title="合同约束"
            subtitle="合同模板管理"
            hint="今日屏使用 master 自带的默认合同 (S≤2.5/A≤9/V≤23/G≥80/Y≥15/CSR≥62/M≤13/岩相≤0.15). 自定义合同模板功能开发中."
          />
        )}
        {tab === "history" && (
          <PlaceholderScreen
            title="历史方案"
            hint="每次保存的配煤方案会出现在这里. 支持对比、导出. 功能开发中."
          />
        )}
        {tab === "me" && (
          <PlaceholderScreen
            title="我的"
            subtitle="设置 / 公式校准 / 关于"
            hint="后续会加 CSR 回归校准、数据导出、关于豆哥配煤等设置项."
          />
        )}
      </div>
      <TabBar active={tab} onChange={setTab} />
    </div>
  );
}

export default App;
