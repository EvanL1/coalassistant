import 'package:sqflite/sqflite.dart';

/// 通用 KV 仓储, 对应 user_settings 表. 用于 coal_prefs /
/// user_contract / history 这类语义化 key/value (跟 D1 settings 表
/// 同一套 key 约定, 方便后续做云同步).
class SettingsRepository {
  SettingsRepository(this._db);

  final Database _db;

  Future<String?> get(String key) async {
    final rows = await _db.query(
      'user_settings',
      columns: ['value'],
      where: 'key = ?',
      whereArgs: [key],
      limit: 1,
    );
    if (rows.isEmpty) return null;
    return rows.first['value'] as String?;
  }

  Future<void> put(String key, String value) async {
    await _db.insert(
      'user_settings',
      {'key': key, 'value': value},
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
  }

  Future<void> remove(String key) async {
    await _db.delete('user_settings', where: 'key = ?', whereArgs: [key]);
  }
}
