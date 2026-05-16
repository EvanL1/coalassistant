import 'dart:convert';

import 'package:sqflite/sqflite.dart';

import '../models/business.dart';

/// 历史方案 仓储. 存在 user_settings KV 表里 (key = 'history'),
/// value 是 JSON 序列化后的 `List<HistoryEntry>`. 跟 storage.ts
/// 的 appendHistory 语义对齐: 最新在前, 最多 100 条 (LRU 截断).
class HistoryRepository {
  HistoryRepository(this._db);

  final Database _db;

  static const String _key = 'history';
  static const int _maxEntries = 100;

  Future<List<HistoryEntry>> list() async {
    final raw = await _readRaw();
    if (raw == null || raw.isEmpty) return const [];
    try {
      final decoded = jsonDecode(raw);
      if (decoded is List) {
        return decoded
            .whereType<Map>()
            .map((m) => _entryFromJson(m.cast<String, Object?>()))
            .toList();
      }
    } catch (_) {
      // fall through
    }
    return const [];
  }

  Future<void> add(HistoryEntry e) async {
    final all = List<HistoryEntry>.from(await list());
    all.insert(0, e); // 最新在前
    if (all.length > _maxEntries) {
      all.removeRange(_maxEntries, all.length);
    }
    await _writeRaw(jsonEncode(all.map(_entryToJson).toList()));
  }

  Future<void> remove(String id) async {
    final all = await list();
    final next = all.where((e) => e.id != id).toList();
    await _writeRaw(jsonEncode(next.map(_entryToJson).toList()));
  }

  Future<String?> _readRaw() async {
    final rows = await _db.query(
      'user_settings',
      columns: ['value'],
      where: 'key = ?',
      whereArgs: [_key],
      limit: 1,
    );
    if (rows.isEmpty) return null;
    return rows.first['value'] as String?;
  }

  Future<void> _writeRaw(String value) async {
    await _db.insert(
      'user_settings',
      {'key': _key, 'value': value},
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
  }

  static HistoryEntry _entryFromJson(Map<String, Object?> m) => HistoryEntry(
        id: m['id'] as String,
        title: (m['title'] ?? '') as String,
        recipeJson: (m['recipe_json'] ?? '') as String,
        costCifPerTon: (m['cost_cif_per_ton'] as num?)?.toDouble() ?? 0,
        totalTons: (m['total_tons'] as num?)?.toDouble(),
        note: m['note'] as String?,
        createdAt: (m['created_at'] ?? '') as String,
      );

  static Map<String, Object?> _entryToJson(HistoryEntry e) => {
        'id': e.id,
        'title': e.title,
        'recipe_json': e.recipeJson,
        'cost_cif_per_ton': e.costCifPerTon,
        'total_tons': e.totalTons,
        'note': e.note,
        'created_at': e.createdAt,
      };
}
