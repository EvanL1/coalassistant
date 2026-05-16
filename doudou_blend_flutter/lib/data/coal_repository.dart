import 'dart:convert';

import 'package:sqflite/sqflite.dart';

import '../models/coal.dart';

class CoalRepository {
  CoalRepository(this._db);

  final Database _db;

  Future<List<MasterCoalEntry>> list() async {
    final rows = await _db.query('user_coals', orderBy: 'updated_at DESC');
    return rows.map(_fromRow).toList();
  }

  /// 插入 / 重名报错抛 [DuplicateCoalException].
  Future<void> add(MasterCoalEntry c) async {
    try {
      await _db.insert('user_coals', {
        'name': c.name.trim(),
        'region': c.region,
        'coal_type': c.coalType,
        'status': c.status.code,
        'props_json': jsonEncode(c.props),
        'fob': c.fob,
        'frt': c.frt,
        'note': c.note,
      });
    } on DatabaseException catch (e) {
      if (e.isUniqueConstraintError()) {
        throw DuplicateCoalException(c.name);
      }
      rethrow;
    }
  }

  /// 强写 (覆盖). 用于 master 同步.
  Future<void> upsert(MasterCoalEntry c) async {
    await _db.insert(
      'user_coals',
      {
        'name': c.name.trim(),
        'region': c.region,
        'coal_type': c.coalType,
        'status': c.status.code,
        'props_json': jsonEncode(c.props),
        'fob': c.fob,
        'frt': c.frt,
        'note': c.note,
      },
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
  }

  Future<void> remove(String name) async {
    await _db.delete('user_coals', where: 'name = ?', whereArgs: [name]);
  }

  static MasterCoalEntry _fromRow(Map<String, Object?> r) {
    Map<String, double> props = const {};
    final raw = r['props_json'] as String?;
    if (raw != null && raw.isNotEmpty) {
      try {
        final decoded = jsonDecode(raw);
        if (decoded is Map) {
          props = {
            for (final e in decoded.entries)
              e.key.toString(): (e.value as num).toDouble()
          };
        }
      } catch (_) {
        props = const {};
      }
    }
    return MasterCoalEntry(
      name: r['name'] as String,
      region: r['region'] as String?,
      coalType: r['coal_type'] as String?,
      status: CoalStatus.fromCode((r['status'] ?? 'draft') as String),
      props: props,
      fob: (r['fob'] as num?)?.toDouble(),
      frt: (r['frt'] as num?)?.toDouble(),
      note: r['note'] as String?,
    );
  }
}

class DuplicateCoalException implements Exception {
  DuplicateCoalException(this.name);
  final String name;
  @override
  String toString() => '煤种已存在: $name';
}
