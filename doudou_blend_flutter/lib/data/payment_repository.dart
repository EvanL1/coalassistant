import 'package:sqflite/sqflite.dart';

import '../models/business.dart';

class PaymentRepository {
  PaymentRepository(this._db);

  final Database _db;

  Future<List<Payment>> list({String? contractId}) async {
    final rows = await _db.query(
      'payments',
      where: contractId == null ? null : 'contract_id = ?',
      whereArgs: contractId == null ? null : [contractId],
      orderBy: 'paid_at DESC',
    );
    return rows.map(_fromRow).toList();
  }

  /// 注意: amount = 0 是合法值 (不能用 falsy 判空, 否则 0 元收款会被
  /// 当成"缺字段". 这是 worker 早期 bug, 已修复, 这里别再踩同样的坑).
  Future<void> upsert(Payment p) async {
    await _db.insert(
      'payments',
      _toRow(p),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
  }

  Future<void> remove(String id) async {
    await _db.delete('payments', where: 'id = ?', whereArgs: [id]);
  }

  static Payment _fromRow(Map<String, Object?> r) {
    return Payment(
      id: r['id'] as String,
      contractId: r['contract_id'] as String,
      kind: PaymentKind.fromCode((r['kind'] ?? 'first') as String),
      amount: (r['amount'] as num).toDouble(),
      paidAt: r['paid_at'] as String,
      payer: r['payer'] as String?,
      method: r['method'] as String?,
      voucherNo: r['voucher_no'] as String?,
      note: r['note'] as String?,
      createdAt: r['created_at'] as String?,
    );
  }

  static Map<String, Object?> _toRow(Payment p) => {
        'id': p.id,
        'contract_id': p.contractId,
        'kind': p.kind.code,
        'amount': p.amount,
        'paid_at': p.paidAt,
        'payer': p.payer,
        'method': p.method,
        'voucher_no': p.voucherNo,
        'note': p.note,
      };
}
