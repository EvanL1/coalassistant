"""在真 E2B 沙箱里验证 doudou-blend 模板: blend 能调用, 结果与本地核心一致.

用法: uv run --with e2b python e2b/smoke.py [模板名]
"""

import json
import sys

from e2b import Sandbox

TEMPLATE = sys.argv[1] if len(sys.argv) > 1 else "doudou-blend"

# 一份有实测对照的配方: 2:4:3:2, 配合煤化验 S 1.30 / A 10.20 / Vdaf 22.14 / G 87.
COALS = [
    ("荣欣", 0.4, 9.7, 24, 97, 23, 75),
    ("新星", 1.6, 10.5, 20, 75, 17, 65),
    ("北沟", 1.8, 8.6, 24, 94, 22, 55),
    ("浮精", 0.8, 12.0, 24, 80, 20, 58),
]
SHARES = {"荣欣": 2, "新星": 4, "北沟": 3, "浮精": 2}
EXPECTED = {"S": 14.2 / 11, "A": 111.2 / 11, "V": 248 / 11, "G": 936 / 11, "CSR": 691 / 11}


def run(sbx: Sandbox, cmd: str):
    # 非零退出码在 SDK 里会抛异常; 这里要自己看退出码, 所以关掉.
    result = sbx.commands.run(cmd, timeout=60, on_stderr=lambda _: None)
    return result


def main() -> None:
    sbx = Sandbox.create(TEMPLATE, timeout=300)
    try:
        master = json.loads(run(sbx, "blend master").stdout)
        specs = master["default_contract"]["specs"]
        for spec in specs:
            if spec["indicator"] == "M":  # 这组数据没有水分
                spec["enabled"] = False

        request = {
            "coals": [
                {
                    "name": name,
                    "fob": 0,
                    "frt": 0,
                    "props": {"S": s, "A": a, "V": v, "G": g, "Y": y, "petro": 0.1, "CSR": csr},
                }
                for name, s, a, v, g, y, csr in COALS
            ],
            "specs": specs,
            "total_quantity": None,
            "truncate_decimal": True,
            "fixed_ratios": SHARES,
        }
        sbx.files.write("/home/user/req.json", json.dumps(request, ensure_ascii=False))
        out = run(sbx, "blend eval < /home/user/req.json")
        assert out.exit_code == 0, f"blend eval 退出码 {out.exit_code}: {out.stderr}"
        result = json.loads(out.stdout)
        assert result["ok"], result.get("reason")

        values = {check["indicator"]: check["value"] for check in result["indicator_check"]}
        for key, expected in EXPECTED.items():
            assert abs(values[key] - expected) < 1e-9, f"{key}: 得 {values[key]}, 应 {expected}"

        usage = sbx.commands.run("blend 2>&1; echo exit=$?", timeout=30)
        assert "exit=2" in usage.stdout, usage.stdout
        readme = run(sbx, "head -1 /home/user/README.md").stdout.strip()
        assert readme == "# 豆哥配煤环境", readme

        print(f"✓ 模板 {TEMPLATE} 冒烟通过")
        for check in result["indicator_check"]:
            print(f"  {check['label_zh']:6} {check['value']:8.3f}  {check['status']}")
        print(f"  quality_status = {result['quality_status']}")
    finally:
        sbx.kill()


if __name__ == "__main__":
    main()
