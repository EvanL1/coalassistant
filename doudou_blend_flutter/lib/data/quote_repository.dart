import 'dart:convert';

import 'package:sqflite/sqflite.dart';

import '../models/business.dart';

class QuoteRepository {
  QuoteRepository(this._db);

  final Database _db;

  Future<List<Quote>> list() async {
    final rows = await _db.query('quotes', orderBy: 'updated_at DESC');
    return rows.map(_fromRow).toList();
  }

  Future<void> upsert(Quote q) async {
    await _db.insert(
      'quotes',
      _toRow(q),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
    // sqflite 的 replace 会重置 updated_at, 手动 patch 一下
    await _db.rawUpdate(
      "UPDATE quotes SET updated_at = datetime('now') WHERE id = ?",
      [q.id],
    );
  }

  Future<void> remove(String id) async {
    await _db.delete('quotes', where: 'id = ?', whereArgs: [id]);
  }

  static Quote _fromRow(Map<String, Object?> r) {
    return Quote(
      id: r['id'] as String,
      customerId: r['customer_id'] as String,
      customerName: r['customer_name'] as String,
      recipe: _decodeRecipe(r['recipe_json'] as String?),
      costCif: (r['cost_cif'] as num).toDouble(),
      markup: (r['markup'] as num?)?.toDouble() ?? 0,
      quotedPrice: (r['quoted_price'] as num).toDouble(),
      totalTons: (r['total_tons'] as num?)?.toDouble(),
      contractName: r['contract_name'] as String?,
      status: QuoteStatus.fromCode((r['status'] ?? 'draft') as String),
      note: r['note'] as String?,
      createdAt: r['created_at'] as String?,
      updatedAt: r['updated_at'] as String?,
    );
  }

  static Map<String, Object?> _toRow(Quote q) => {
        'id': q.id,
        'customer_id': q.customerId,
        'customer_name': q.customerName,
        'recipe_json': jsonEncode(q.recipe),
        'cost_cif': q.costCif,
        'markup': q.markup,
        'quoted_price': q.quotedPrice,
        'total_tons': q.totalTons,
        'contract_name': q.contractName,
        'status': q.status.code,
        'note': q.note,
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
