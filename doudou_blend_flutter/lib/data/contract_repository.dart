import 'dart:convert';

import 'package:sqflite/sqflite.dart';

import '../models/business.dart';

class ContractRepository {
  ContractRepository(this._db);

  final Database _db;

  Future<List<Contract>> list() async {
    final rows = await _db.query('contracts', orderBy: 'updated_at DESC');
    return rows.map(_fromRow).toList();
  }

  Future<void> upsert(Contract c) async {
    await _db.insert(
      'contracts',
      _toRow(c),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
    await _db.rawUpdate(
      "UPDATE contracts SET updated_at = datetime('now') WHERE id = ?",
      [c.id],
    );
  }

  /// 级联删除: contracts + 关联的 payments / shipments. 跟 worker
  /// handleDeleteContract 一致 (D1 默认不开 FK cascade, 手动 batch).
  Future<void> remove(String id) async {
    await _db.transaction((txn) async {
      await txn
          .delete('payments', where: 'contract_id = ?', whereArgs: [id]);
      await txn
          .delete('shipments', where: 'contract_id = ?', whereArgs: [id]);
      await txn.delete('contracts', where: 'id = ?', whereArgs: [id]);
    });
  }

  static Contract _fromRow(Map<String, Object?> r) {
    return Contract(
      id: r['id'] as String,
      quoteId: r['quote_id'] as String?,
      customerId: r['customer_id'] as String,
      customerName: r['customer_name'] as String,
      contractNo: r['contract_no'] as String?,
      billingLocation: r['billing_location'] as String?,
      prepayParty: r['prepay_party'] as String?,
      recipe: _decodeRecipe(r['recipe_json'] as String?),
      unitPrice: (r['unit_price'] as num).toDouble(),
      totalTons: (r['total_tons'] as num).toDouble(),
      totalAmount: (r['total_amount'] as num).toDouble(),
      firstPayPct: (r['first_pay_pct'] as num?)?.toDouble() ?? 80,
      firstPayAmount: (r['first_pay_amount'] as num).toDouble(),
      tailPayAmount: (r['tail_pay_amount'] as num).toDouble(),
      signedAt: r['signed_at'] as String?,
      status: ContractStatus.fromCode((r['status'] ?? 'active') as String),
      note: r['note'] as String?,
      createdAt: r['created_at'] as String?,
      updatedAt: r['updated_at'] as String?,
    );
  }

  static Map<String, Object?> _toRow(Contract c) => {
        'id': c.id,
        'quote_id': c.quoteId,
        'customer_id': c.customerId,
        'customer_name': c.customerName,
        'contract_no': c.contractNo,
        'billing_location': c.billingLocation,
        'prepay_party': c.prepayParty,
        'recipe_json': jsonEncode(c.recipe),
        'unit_price': c.unitPrice,
        'total_tons': c.totalTons,
        'total_amount': c.totalAmount,
        'first_pay_pct': c.firstPayPct,
        'first_pay_amount': c.firstPayAmount,
        'tail_pay_amount': c.tailPayAmount,
        'signed_at': c.signedAt,
        'status': c.status.code,
        'note': c.note,
      };

  static Map<String, double> _decodeRecipe(String? raw) {
    if (raw == null || raw.isEmpty) return const {};
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map) {
        return {
          for (final e in decoded.entries)
            e.key.toString(): (e.value as num).toDouble(),
        };
      }
    } catch (_) {
      // fall through
    }
    return const {};
  }
}
