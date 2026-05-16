/// 加载 assets/coal_master.json. 给 UI 一个 master coal 列表 + 默认合同模板.
library;

import 'dart:convert';

import 'package:flutter/services.dart' show rootBundle;

import '../models/coal.dart';
import '../models/spec.dart';

class CoalMaster {
  CoalMaster({
    required this.version,
    required this.updatedAt,
    required this.description,
    required this.defaultContract,
    required this.coals,
  });

  final String version;
  final String updatedAt;
  final String description;
  final DefaultContract defaultContract;
  final List<MasterCoalEntry> coals;

  /// 从 bundle 加载一次, 缓存在内存.
  static CoalMaster? _cached;

  static Future<CoalMaster> load() async {
    if (_cached != null) return _cached!;
    final raw = await rootBundle.loadString('assets/coal_master.json');
    final j = jsonDecode(raw) as Map<String, dynamic>;
    _cached = CoalMaster(
      version: (j['version'] ?? '') as String,
      updatedAt: (j['updated_at'] ?? '') as String,
      description: (j['description'] ?? '') as String,
      defaultContract: DefaultContract.fromJson(
          j['default_contract'] as Map<String, dynamic>),
      coals: ((j['coals'] as List?) ?? [])
          .map((c) => MasterCoalEntry.fromJson(c as Map<String, dynamic>))
          .toList(),
    );
    return _cached!;
  }

  /// 测试用: 直接喂 JSON 字符串.
  static CoalMaster fromString(String raw) {
    final j = jsonDecode(raw) as Map<String, dynamic>;
    return CoalMaster(
      version: (j['version'] ?? '') as String,
      updatedAt: (j['updated_at'] ?? '') as String,
      description: (j['description'] ?? '') as String,
      defaultContract: DefaultContract.fromJson(
          j['default_contract'] as Map<String, dynamic>),
      coals: ((j['coals'] as List?) ?? [])
          .map((c) => MasterCoalEntry.fromJson(c as Map<String, dynamic>))
          .toList(),
    );
  }
}
