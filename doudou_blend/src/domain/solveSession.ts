import type { BlendRequest, BlendResult } from "../types";
import type { DriftSummary } from "./resolvedCoal";

export interface SolveSnapshot {
  requestId: number;
  request: BlendRequest;
  result: BlendResult;
  contractName: string;
  enabledCount: number;
  /** 本次求解用了推算价时的说明; null = 全部按录入价 */
  drift?: DriftSummary | null;
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
