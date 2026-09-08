import type { BlendRequest, BlendResult } from "../types";
import type { PriceStatus } from "./resolvedCoal";

export interface SolveSnapshot {
  requestId: number;
  request: BlendRequest;
  result: BlendResult;
  contractName: string;
  enabledCount: number;
  /** 本次求解所用价格的可信度摘要(报价时效 + 是否推算) */
  price?: PriceStatus | null;
}

export class LatestRequestTracker {
  private current = 0;

  get currentRequestId(): number {
    return this.current;
  }

  issue(): number {
    this.current += 1;
    return this.current;
  }

  invalidate(): void {
    this.current += 1;
  }

  isCurrent(requestId: number): boolean {
    return requestId === this.current;
  }

  accept(requestId: number, apply: () => void): boolean {
    if (!this.isCurrent(requestId)) return false;
    apply();
    return true;
  }
}

export function isSnapshotActionable(
  snapshot: SolveSnapshot | null,
  currentRequestId: number,
  qtyInput: string,
): boolean {
  if (
    snapshot == null ||
    snapshot.requestId !== currentRequestId ||
    !snapshot.result.ok
  ) {
    return false;
  }

  const inputQuantity = Number(qtyInput);
  const solvedQuantity = snapshot.request.total_quantity;
  return (
    qtyInput.trim() !== "" &&
    Number.isFinite(inputQuantity) &&
    inputQuantity > 0 &&
    typeof solvedQuantity === "number" &&
    Number.isFinite(solvedQuantity) &&
    solvedQuantity > 0 &&
    inputQuantity === solvedQuantity
  );
}
