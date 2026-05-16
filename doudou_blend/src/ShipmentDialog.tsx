/**
 * 发货 / 到货 / 结算 录入.
 * 入口: ContractDetailDialog "记一笔发货" 按钮.
 *
 * 一个 shipment 三阶段:
 *   shipped → 加 vehicle_no + net_tons + shipped_at
 *   arrived → 加 arrived_at
 *   settled → 加 settled_at + settled_amount + assay (化验)
 */
import { useEffect, useState } from "react";
import type { Shipment, ShipmentStatus } from "./types";
import { apiUpsertShipment } from "./api";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "./types";

interface Props {
  contractId: string;
  editing?: Shipment | null;
  onClose: () => void;
  onSaved?: () => void;
}

const STATUS_LABEL: Record<ShipmentStatus, string> = {
  shipped: "已发出",
  arrived: "已到货",
  settled: "已结算",
};

function newId(): string {
  return (
    (globalThis.crypto?.randomUUID && globalThis.crypto.randomUUID()) ||
    `s_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
  );
}

function todayISO(): string {
  return new Date().toISOString().slice(0, 10);
}

export function ShipmentDialog({ contractId, editing, onClose, onSaved }: Props) {
  const [vehicleNo, setVehicleNo] = useState(editing?.vehicle_no ?? "");
  const [netTons, setNetTons] = useState(
    editing?.net_tons != null ? String(editing.net_tons) : "",
  );
  const [grossTons, setGrossTons] = useState(
    editing?.gross_tons != null ? String(editing.gross_tons) : "",
  );
  const [tareTons, setTareTons] = useState(
    editing?.tare_tons != null ? String(editing.tare_tons) : "",
  );
  const [shippedAt, setShippedAt] = useState(editing?.shipped_at ?? todayISO());
  const [arrivedAt, setArrivedAt] = useState(editing?.arrived_at ?? "");
  const [settledAt, setSettledAt] = useState(editing?.settled_at ?? "");
  const [settledAmount, setSettledAmount] = useState(
    editing?.settled_amount != null ? String(editing.settled_amount) : "",
  );
  const [status, setStatus] = useState<ShipmentStatus>(editing?.status ?? "shipped");
  const [note, setNote] = useState(editing?.note ?? "");
  const [assay, setAssay] = useState<Record<string, string>>(
    Object.fromEntries(
      INDICATOR_ORDER.map((k) => [k, editing?.assay?.[k] != null ? String(editing.assay[k]) : ""]),
    ),
  );
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  const netNum = parseFloat(netTons) || 0;
  const canSubmit = netNum > 0 && shippedAt.trim() !== "" && !submitting;

  function buildAssay(): Partial<Record<string, number>> | null {
    const out: Record<string, number> = {};
    let any = false;
    for (const k of INDICATOR_ORDER) {
      const v = parseFloat(assay[k] ?? "");
      if (Number.isFinite(v)) {
        out[k] = v;
        any = true;
      }
    }
    return any ? out : null;
  }

  async function submit() {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const s: Shipment = {
      id: editing?.id ?? newId(),
      contract_id: contractId,
      vehicle_no: vehicleNo.trim() || null,
      net_tons: netNum,
      gross_tons: grossTons ? parseFloat(grossTons) : null,
      tare_tons: tareTons ? parseFloat(tareTons) : null,
      shipped_at: shippedAt,
      arrived_at: arrivedAt || null,
      settled_at: settledAt || null,
      settled_amount: settledAmount ? parseFloat(settledAmount) : null,
      assay: buildAssay(),
      status,
      note: note.trim() || null,
    };
    try {
      await apiUpsertShipment(s);
      onSaved?.();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSubmitting(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal-sheet" onClick={(e) => e.stopPropagation()}>
        <div className="modal-handle" />
        <div className="modal-header">
          <div>
            <div className="modal-title">{editing ? "编辑发货" : "记一笔发货"}</div>
            <div className="modal-subtitle">
              {netNum} 吨 · {STATUS_LABEL[status]}
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>

        <div className="modal-body">
          <div className="edit-section">
            <div className="edit-section-title">状态</div>
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              {(["shipped", "arrived", "settled"] as ShipmentStatus[]).map((s) => (
                <button
                  key={s}
                  onClick={() => setStatus(s)}
                  style={{
                    padding: "6px 14px",
                    borderRadius: 999,
                    fontSize: 13,
                    background: status === s ? "var(--c-primary)" : "var(--c-bg)",
                    color: status === s ? "white" : "var(--c-text-2)",
                    fontWeight: status === s ? 600 : 400,
                  }}
                >
                  {STATUS_LABEL[s]}
                </button>
              ))}
            </div>
          </div>

          <div className="edit-section">
            <div className="edit-section-title">基本</div>
            <div className="edit-row">
              <div className="edit-row-label">车号 / 船号</div>
              <input
                type="text"
                className="edit-input"
                value={vehicleNo ?? ""}
                onChange={(e) => setVehicleNo(e.target.value)}
                placeholder="如 鲁A12345"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">
                净重 (吨) <span style={{ color: "var(--c-danger)" }}>*</span>
              </div>
              <input
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={netTons}
                onChange={(e) => setNetTons(e.target.value)}
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">毛重 (吨)</div>
              <input
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={grossTons}
                onChange={(e) => setGrossTons(e.target.value)}
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">皮重 (吨)</div>
              <input
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={tareTons}
                onChange={(e) => setTareTons(e.target.value)}
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">
                发货日 <span style={{ color: "var(--c-danger)" }}>*</span>
              </div>
              <input
                type="date"
                className="edit-input"
                value={shippedAt}
                onChange={(e) => setShippedAt(e.target.value)}
              />
            </div>
            {(status === "arrived" || status === "settled") && (
              <div className="edit-row">
                <div className="edit-row-label">到货日</div>
                <input
                  type="date"
                  className="edit-input"
                  value={arrivedAt ?? ""}
                  onChange={(e) => setArrivedAt(e.target.value)}
                />
              </div>
            )}
            {status === "settled" && (
              <>
                <div className="edit-row">
                  <div className="edit-row-label">结算日</div>
                  <input
                    type="date"
                    className="edit-input"
                    value={settledAt ?? ""}
                    onChange={(e) => setSettledAt(e.target.value)}
                  />
                </div>
                <div className="edit-row">
                  <div className="edit-row-label">结算金额</div>
                  <input
                    type="number"
                    inputMode="decimal"
                    className="edit-input"
                    value={settledAmount ?? ""}
                    onChange={(e) => setSettledAmount(e.target.value)}
                    placeholder="元"
                  />
                </div>
              </>
            )}
          </div>

          {(status === "arrived" || status === "settled") && (
            <div className="edit-section">
              <div className="edit-section-title">化验值 (可选)</div>
              {INDICATOR_ORDER.map((k) => (
                <div className="edit-row" key={k}>
                  <div className="edit-row-label">{INDICATOR_LABEL[k]}</div>
                  <input
                    type="number"
                    inputMode="decimal"
                    className="edit-input"
                    value={assay[k] ?? ""}
                    onChange={(e) =>
                      setAssay({ ...assay, [k]: e.target.value })
                    }
                  />
                </div>
              ))}
            </div>
          )}

          <div className="edit-section">
            <div className="edit-row">
              <div className="edit-row-label">备注</div>
              <input
                type="text"
                className="edit-input"
                value={note ?? ""}
                onChange={(e) => setNote(e.target.value)}
              />
            </div>
          </div>

          {error && (
            <div
              style={{
                color: "var(--c-danger)",
                fontSize: 12,
                marginTop: 8,
                padding: "8px 12px",
                background: "rgba(220, 38, 38, 0.08)",
                borderRadius: 8,
              }}
            >
              保存失败: {error}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>取消</button>
          <button
            className="btn btn-primary"
            onClick={submit}
            disabled={!canSubmit}
            style={{ opacity: canSubmit ? 1 : 0.4 }}
          >
            {submitting ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
