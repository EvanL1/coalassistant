"""拉钢铁网炼焦煤指数 (MCCI) 的多年日度历史, 存成 CSV 给预测回测用.

与 blend_kit_server/src/mcci.rs 同一个公开接口、同一套签名, 只是窗口更长 (线上只回填一年).
数据不进仓库 (仓库公开, 指数是钢铁网的数据).

用法: python3 -I fetch_history.py <起始年> <输出.csv>
"""
import csv
import hashlib
import json
import sys
import time
import urllib.parse
import urllib.request
from datetime import date

BASE = "https://index.mysteel.com"
PATH = "/zs/newxpic/getReport.ms"
APP_KEY = "47EE3F12CF0C443F8FD51EFDA73AC815"
APP_SECRET = "3BA6477330684B19AA6AF4485497B5F2"
VERSION = "1.0.0"
SERIES = [
    ("LJM_ZH", "综合炼焦煤"),
    ("LJM_DLZJM", "低硫主焦煤"),
    ("LJM_ZLZJM", "中硫主焦煤"),
    ("LJM_GLZJM", "高硫主焦煤"),
    ("LJM_FM", "肥煤"),
    ("LJM_OTJM", "1/3焦煤"),
    ("LJM_SM", "瘦煤"),
    ("LJM_QM", "气煤"),
]
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def fetch(code: str, name: str, start: date, end: date) -> list[tuple[str, float]]:
    ts = int(time.time() * 1000)
    sign = hashlib.md5(f"path{PATH}timestamp{ts}version{VERSION}{APP_SECRET}".encode()).hexdigest().upper()
    query = urllib.parse.urlencode({
        "typeName": name, "tabName": code, "dateType": "day",
        "startTime": start.isoformat(), "endTime": end.isoformat(),
        "returnType": "", "callback": "json", "v": ts,
    })
    request = urllib.request.Request(f"{BASE}{PATH}?{query}", headers={
        "version": VERSION, "appKey": APP_KEY, "timestamp": str(ts), "sign": sign,
        "Referer": f"{BASE}/xpic/detail.html?tabName=ljm", "User-Agent": "Mozilla/5.0",
    })
    with OPENER.open(request, timeout=20) as response:
        report = json.loads(response.read().decode("utf-8"))
    if report.get("code") not in (None, 200):
        raise RuntimeError(f"{code} {start}: {report.get('code')} {report.get('message')}")
    rows = []
    for row in report.get("data") or []:
        try:
            rows.append((row["date"].replace("/", "-"), float(row["value"])))
        except (KeyError, ValueError):
            continue
    return rows


def main() -> None:
    first_year, out = int(sys.argv[1]), sys.argv[2]
    today = date.today()
    seen: dict[tuple[str, str], float] = {}
    for code, name in SERIES:
        for year in range(first_year, today.year + 1):
            start, end = date(year, 1, 1), min(date(year, 12, 31), today)
            rows = fetch(code, name, start, end)
            for day, value in rows:
                seen[(code, day)] = value
            print(f"{code} {year}: {len(rows)} 条", file=sys.stderr)
            time.sleep(2)
    with open(out, "w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["series", "date", "value"])
        for (code, day), value in sorted(seen.items()):
            writer.writerow([code, day, value])


if __name__ == "__main__":
    main()
