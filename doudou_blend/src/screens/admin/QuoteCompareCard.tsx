/**
 * 询价对比: 用户询价精算时存档的出厂价 vs 服务端同口径估价, 按煤种汇总 + 明细.
 * 汇总取中位数, 是以后校准估价系数的依据; 试用或填错的询价在这里删掉, 不进汇总.
 */
import { useEffect, useRef, useState } from "react";
import { deleteQuote, fetchQuoteComparison } from "../../admin";
import type { QuoteComparisonResponse, QuoteRecord } from "../../types";
import { cellStyle, errorMessage, errorText, smallButton } from "./styles";

const SUMMARY_COLUMNS = ["煤种", "条数", "报价/估价中位", "差额中位"];
const DETAIL_COLUMNS = [
  "日期", "煤", "煤种", "对标基准", "出厂", "运费", "估价", "差额", "比例", "改过的指标", "操作",
];
const tableStyle = { width: "100%", borderCollapse: "collapse", fontSize: 12 } as const;

const signed = (value: number | null) =>
  value == null ? "—" : `${value > 0 ? "+" : ""}${Math.round(value)}`;

export function QuoteCompareCard() {
  const [data, setData] = useState<Omit<QuoteComparisonResponse, "ok"> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const mounted = useRef(true);
  const deletingRef = useRef(false);

  useEffect(() => {
    let alive = true;
    mounted.current = true;
    void fetchQuoteComparison()
      .then((result) => {
        if (alive) setData(result);
      })
      .catch((error: unknown) => {
        if (alive) setError(errorMessage(error));
      });
    return () => {
      alive = false;
      mounted.current = false;
    };
  }, []);

  async function remove(id: string) {
    if (deletingRef.current || !confirm("删除这条询价? 删除后不进汇总")) return;
    deletingRef.current = true;
    setDeleting(id);
    setError(null);
    try {
      await deleteQuote(id);
      if (!mounted.current) return;
      setData(null);
      const result = await fetchQuoteComparison();
      if (mounted.current) setData(result);
    } catch (error) {
      if (mounted.current) setError(errorMessage(error));
    } finally {
      deletingRef.current = false;
      if (mounted.current) setDeleting(null);
    }
  }

  return (
    <div className="card">
      <div className="card-title">询价对比</div>
      {error && <div style={errorText}>{error}</div>}
      {!data && !error && (
        <div style={{ fontSize: 12, color: "var(--c-text-3)" }}>加载中...</div>
      )}
      {data &&
        (data.quotes.length === 0 ? (
          <p>还没有询价</p>
        ) : (
          <>
            {data.by_type.length > 0 && (
              <div style={{ overflowX: "auto" }}>
                <table aria-label="询价按煤种汇总" style={tableStyle}>
                  <HeaderRow columns={SUMMARY_COLUMNS} />
                  <tbody>
                    {data.by_type.map((group) => (
                      <tr key={group.coal_type}>
                        <td style={cellStyle}>{group.coal_type}</td>
                        <td style={cellStyle}>{group.count}</td>
                        <td style={cellStyle}>{group.median_ratio.toFixed(2)}</td>
                        <td style={cellStyle}>{signed(group.median_gap)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            <div style={{ overflowX: "auto", marginTop: 12 }}>
              <table aria-label="询价明细" style={tableStyle}>
                <HeaderRow columns={DETAIL_COLUMNS} />
                <tbody>
                  {data.quotes.map((quote) => (
                    <DetailRow
                      key={quote.id}
                      quote={quote}
                      deleting={deleting}
                      onDelete={() => void remove(quote.id)}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          </>
        ))}
    </div>
  );
}

function HeaderRow({ columns }: { columns: string[] }) {
  return (
    <thead>
      <tr>
        {columns.map((label) => (
          <th key={label} style={cellStyle}>
            {label}
          </th>
        ))}
      </tr>
    </thead>
  );
}

function DetailRow({
  quote,
  deleting,
  onDelete,
}: {
  quote: QuoteRecord;
  /** 正在删的那条; 删除期间所有删除按钮都禁用. */
  deleting: string | null;
  onDelete: () => void;
}) {
  return (
    <tr>
      <td style={cellStyle}>{quote.quoted_on}</td>
      <td style={cellStyle}>{quote.name}</td>
      <td style={cellStyle}>{quote.coal_type?.trim() || "未标煤种"}</td>
      <td style={cellStyle}>{quote.benchmark ?? "—"}</td>
      <td style={cellStyle}>{Math.round(quote.fob)}</td>
      <td style={cellStyle}>{Math.round(quote.frt)}</td>
      <td style={cellStyle}>{quote.estimate == null ? "—" : Math.round(quote.estimate)}</td>
      <td style={cellStyle}>{signed(quote.gap)}</td>
      <td style={cellStyle}>{quote.ratio == null ? "—" : quote.ratio.toFixed(2)}</td>
      <td style={cellStyle}>{quote.changed.join(", ") || "—"}</td>
      <td style={cellStyle}>
        <button
          className="btn btn-secondary"
          style={smallButton}
          disabled={deleting !== null}
          onClick={onDelete}
        >
          {deleting === quote.id ? "删除中..." : "删除"}
        </button>
      </td>
    </tr>
  );
}
