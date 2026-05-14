//! 用 master 数据库的 4 种 verified 主力煤 + 默认合同跑配煤求解.
//! 这是 APP 首次启动后的"out-of-the-box"场景.
use blend_kit::{solve, BlendRequest, CoalMaster, MasterStatus};

fn main() {
    println!("===== 加载 master =====");
    let master = CoalMaster::load_embedded().expect("master 加载失败");
    println!("版本: {} (更新于 {})", master.version, master.updated_at);
    println!("总煤种: {}", master.coals.len());
    println!(
        "  verified:   {} (生产可用)",
        master.verified().count()
    );
    println!(
        "  active:     {} (部分数据)",
        master.by_status(MasterStatus::Active).count()
    );
    println!(
        "  draft:      {} (待核实)",
        master.by_status(MasterStatus::Draft).count()
    );
    println!(
        "  incomplete: {} (未录入)",
        master.by_status(MasterStatus::Incomplete).count()
    );
    println!(
        "  archived:   {} (已停用)",
        master.by_status(MasterStatus::Archived).count()
    );
    println!();

    println!("===== 主力煤池 (4 verified, 完整 10 字段) =====");
    let coals: Vec<_> = master
        .verified()
        .filter_map(|e| {
            println!(
                "  {:10} S={:.2} A={:.1} V={:.1} G={:.0} Y={:.0} CSR={:.0} M={:.1} ¥{:.0}+{:.0}",
                e.name,
                e.props["S"], e.props["A"], e.props["V"], e.props["G"],
                e.props["Y"], e.props["CSR"], e.props["M"],
                e.fob.unwrap(), e.frt.unwrap()
            );
            e.to_coal(None, None)
        })
        .collect();
    println!();

    println!("===== 默认合同 (master 自带) =====");
    println!("'{}':", master.default_contract.name);
    for s in &master.default_contract.specs {
        let constraint = match (s.min, s.max) {
            (Some(lo), Some(hi)) => format!("∈[{}, {}]", lo, hi),
            (Some(lo), None) => format!("≥ {}", lo),
            (None, Some(hi)) => format!("≤ {}", hi),
            (None, None) => "无约束".into(),
        };
        println!("  {} {}", s.indicator, constraint);
    }
    println!();

    let req = BlendRequest {
        coals,
        specs: master.default_contract.specs.clone(),
        total_quantity: Some(3700.0),
        truncate_decimal: true,
    };
    let r = solve(&req);

    println!("===== 求解结果 =====");
    if !r.ok {
        println!("❌ 不可行: {}", r.reason.unwrap());
        for w in &r.warnings {
            println!("  ⚠ {}", w);
        }
        return;
    }

    let cost = r.cost.unwrap();
    println!("最低到厂价: {:.2} 元/吨", cost.cif_per_ton);
    println!("  其中 FOB: {:.2}", cost.fob_per_ton);
    println!("  其中 FRT: {:.2}", cost.frt_per_ton);
    println!("总订单金额: {:.2} 元 (3700 吨)", cost.total_cif.unwrap());
    println!();

    println!("订单明细:");
    for o in &r.orders {
        println!(
            "  {:10} {:>6.2}%  {:>8.2} 吨  ¥{:>10.2}",
            o.coal,
            o.ratio * 100.0,
            o.tons.unwrap_or(0.0),
            o.cif_amount.unwrap_or(0.0)
        );
    }
    println!();

    println!("8 项指标体检:");
    for ic in &r.indicator_check {
        let range = match (ic.min, ic.max) {
            (Some(lo), Some(hi)) => format!("[{},{}]", lo, hi),
            (Some(lo), None) => format!("≥{}", lo),
            (None, Some(hi)) => format!("≤{}", hi),
            (None, None) => "—".into(),
        };
        let status = if ic.binding {
            "★ 顶格"
        } else if ic.slack.map(|s| s < -1e-6).unwrap_or(false) {
            "✗ 违反"
        } else {
            "✓"
        };
        println!(
            "  {:10} {:>7.3}  {:>10}  {}",
            ic.label_zh, ic.value, range, status
        );
    }

    let binding: Vec<_> = r.indicator_check.iter().filter(|c| c.binding).collect();
    if !binding.is_empty() {
        println!();
        println!("谈判方向 (binding):");
        for ic in &binding {
            println!("  {} 顶格 → 找{}{}的煤源, 或谈宽合同",
                ic.label_zh,
                if ic.max.is_some() { "更低" } else { "更高" },
                ic.label_zh);
        }
    }
}
