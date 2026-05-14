import { useState } from "react";
import "./App.css";
import { TabBar, type TabId } from "./TabBar";
import { TodayScreen } from "./screens/TodayScreen";
import { CoalPoolScreen } from "./screens/CoalPoolScreen";
import { ContractScreen } from "./screens/ContractScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { PlaceholderScreen } from "./screens/PlaceholderScreen";

function App() {
  const [tab, setTab] = useState<TabId>("today");

  return (
    <div className="app">
      <div className="app-content">
        {tab === "today" && <TodayScreen />}
        {tab === "pool" && <CoalPoolScreen />}
        {tab === "contract" && <ContractScreen />}
        {tab === "history" && <HistoryScreen />}
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
