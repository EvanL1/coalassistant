//! 豆哥配煤 - 8 煤 + 8 指标完整场景 demo.
//! 跑你昨晚验证的架构: FOB/FRT 拆分 + 三视图输出.
use blend_kit::{coal_from_tuple, label_zh, solve, BlendRequest, Spec};
use std::time::Instant;

fn main() {
    // 8 煤池 (S, A, V, G, Y, petro, CSR, M, FOB, FRT)
    let coals = vec![
        coal_from_tuple("铁新",   (3.2, 6.5,  21.0, 92.0,  14.0, 0.10, 62.0, 7.5, 1260.0, 30.0)),
        coal_from_tuple("临北",   (2.0, 6.0,  22.0, 93.0,  16.0, 0.10, 66.0, 7.0, 1250.0, 30.0)),
        coal_from_tuple("安益",   (3.4, 6.8,  18.0, 75.0,  10.0, 0.18, 60.0, 8.0, 1120.0, 30.0)),
        coal_from_tuple("筛精",   (3.9, 9.5,  25.0, 100.0, 22.0, 0.08, 65.0, 9.5, 970.0,  30.0)),
        coal_from_tuple("孟子峪", (2.0, 9.5,  18.0, 75.0,  10.0, 0.18, 58.0, 7.8, 1034.0, 30.0)),
        coal_from_tuple("大佛寺", (3.0, 8.5,  18.0, 75.0,  10.0, 0.16, 59.0, 8.0, 1120.0, 30.0)),
        coal_from_tuple("神州",   (2.6, 10.0, 17.0, 65.0,   8.0, 0.20, 55.0, 7.2, 1110.0, 30.0)),
        coal_from_tuple("豹子沟", (3.8, 11.0, 24.0, 92.0,  20.0, 0.12, 64.0, 8.5, 1250.0, 30.0)),
    ];

    let specs = vec![
        Spec::upper("S", 2.5),
        Spec::upper("A", 9.0),
        Spec::range("V", 18.0, 27.0),
        Spec::lower("G", 80.0),
        Spec::lower("Y", 14.0),
        Spec::upper("M", 12.0),
        Spec::lower("CSR", 60.0),
    ];

    let req = BlendRequest {
        coals,
        specs,
        total_quantity: Some(3700.0),
        truncate_decimal: true,
        csr_observations: None,
    };

    println!("===== 豆哥配煤 Rust 核心 =====");
    println!("8 煤池 + 7 项合同约束 + 总采购 3700 吨");
    println!();

    // 性能基准
    let runs = 100;
    let start = Instant::now();
    let mut last = None;
    for _ in 0..runs {
        last = Some(solve(&req));
    }
    let elapsed = start.elapsed();
    let r = last.unwrap();
    println!(
        "性能: {} 次求解 {:.2}ms 总, 平均 {:.3}ms/次\n",
        runs,
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1000.0 / runs as f64
    );

    if !r.ok {
        println!("❌ {}", r.reason.unwrap());
        for w in &r.warnings {
            println!("⚠️  {}", w);
        }
        return;
    }

    // 视图 A: 成本三层结构
    let cost = r.cost.as_ref().unwrap();
    println!("─── 视图 A · 成本结构 (财务) ───");
    println!("  FOB (出厂) 加权: {:>10.2} 元/吨", cost.fob_per_ton);
    println!("  FRT (运费) 加权: {:>10.2} 元/吨", cost.frt_per_ton);
    println!("  CIF (到厂) 合计: {:>10.2} 元/吨", cost.cif_per_ton);
    if let (Some(tf), Some(tt), Some(tc)) = (cost.total_fob, cost.total_frt, cost.total_cif) {
        println!("  ─────");
        println!("  总 FOB 金额:    {:>12.2} 元", tf);
        println!("  总 FRT 金额:    {:>12.2} 元", tt);
        println!("  总 CIF 金额:    {:>12.2} 元", tc);
    }
    println!();

    // 视图 B: 实物订单
    println!("─── 视图 B · 实物订单 (采购) ───");
    println!(
        "  {:8} {:>7} {:>10} {:>12}",
        "煤名", "配比%", "吨数", "到厂金额"
    );
    for o in &r.orders {
        println!(
            "  {:8} {:>6.2}% {:>10.2} {:>12.2}",
            o.coal,
            o.ratio * 100.0,
            o.tons.unwrap_or(0.0),
            o.cif_amount.unwrap_or(0.0)
        );
    }
    println!();

    // 视图 C: 指标体检 (附 binding 标记)
    println!("─── 视图 C · 指标体检 (质检/销售) ───");
    println!("  {:<10} {:>8} {:>14} {:>10} {}", "指标", "实际值", "范围", "余量", "状态");
    for ic in &r.indicator_check {
        let range = match (ic.min, ic.max) {
            (Some(lo), Some(hi)) => format!("{:.1}~{:.1}", lo, hi),
            (Some(lo), None) => format!("≥{:.1}", lo),
            (None, Some(hi)) => format!("≤{:.1}", hi),
            (None, None) => "—".into(),
        };
        let status = if ic.binding {
            "★ binding (顶格)"
        } else if ic.slack.map(|s| s < -1e-6).unwrap_or(false) {
            "✗ 违反"
        } else {
            "✓"
        };
        let slack_str = ic.slack.map(|s| format!("{:>10.3}", s)).unwrap_or_else(|| "        —".into());
        println!(
            "  {:<10} {:>8.3} {:>14} {} {}",
            ic.label_zh, ic.value, range, slack_str, status
        );
    }
    println!();

    // 谈判方向逆向归因
    let binding_inds: Vec<_> = r.indicator_check.iter().filter(|c| c.binding).collect();
    if !binding_inds.is_empty() {
        println!("─── 谈判方向 (binding 集合逆向归因) ───");
        for ic in binding_inds {
            println!(
                "  {} 顶格 → 找{}{}煤, 或谈宽该项约束",
                ic.label_zh,
                if ic.max.is_some() { "更低" } else { "更高" },
                ic.label_zh
            );
        }
    }

    for w in &r.warnings {
        println!("\n⚠️  {}", w);
    }

    // 通过名称查询中文标签
    let _ = label_zh("S");
}
