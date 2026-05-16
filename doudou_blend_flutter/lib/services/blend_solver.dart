/// 配煤求解器入口.
///
/// 优先用 blend_kit_rs 编出的 cdylib (libblend_kit_rs.so / .dylib /
/// .dll / 鸿蒙 .so) 通过 Dart FFI 调用; cdylib 不可用时 fallback 到
/// 纯 Dart 实现 (only 计算成本, 不做 LP - 给测试 / Web demo 用).
///
/// 真正 LP 求解必须用 Rust 这条路: clarabel / highs 这些 LP 求解器没法
/// 在 Dart 里复刻.
///
/// 鸿蒙真机部署:
///   1. cd /home/user/coalassistant/blend_kit_rs
///   2. cargo build --release --target aarch64-unknown-linux-ohos
///   3. cp target/.../libblend_kit_rs.so → doudou_blend_flutter/ohos/entry/libs/arm64-v8a/
///   4. (DevEco 自动打进 .hap)
library;

import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

import 'package:ffi/ffi.dart';

import '../models/blend.dart';

abstract interface class BlendSolver {
  Future<BlendResult> solve(BlendRequest req);

  /// 选择最佳可用实现: FFI > Dart fallback.
  static BlendSolver create() {
    try {
      return _FfiBlendSolver();
    } catch (_) {
      return DartFallbackBlendSolver();
    }
  }
}

// ============================================================
// FFI 实现
// ============================================================

/// 跟 blend_kit_rs 暴露的 extern "C" fn 对齐:
///
/// ```rust
/// #[no_mangle]
/// pub extern "C" fn blend_solve(input_json: *const c_char) -> *mut c_char {
///     // ... 解析 json → solve → 返回 json
/// }
///
/// #[no_mangle]
/// pub extern "C" fn blend_free_string(s: *mut c_char) {
///     unsafe { let _ = CString::from_raw(s); }
/// }
/// ```
///
/// 当前 blend_kit_rs 还没暴露这两个 fn (本来是 WASM-only). 调用前需要先在
/// blend_kit_rs/src/lib.rs 加 extern "C" 包装. 见 native/README.md.
typedef _BlendSolveC = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>);
typedef _BlendSolveDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>);
typedef _BlendFreeC = ffi.Void Function(ffi.Pointer<Utf8>);
typedef _BlendFreeDart = void Function(ffi.Pointer<Utf8>);

class _FfiBlendSolver implements BlendSolver {
  _FfiBlendSolver() {
    final lib = ffi.DynamicLibrary.open(_libName);
    _solve = lib
        .lookup<ffi.NativeFunction<_BlendSolveC>>('blend_solve')
        .asFunction();
    _free = lib
        .lookup<ffi.NativeFunction<_BlendFreeC>>('blend_free_string')
        .asFunction();
  }

  late final _BlendSolveDart _solve;
  late final _BlendFreeDart _free;

  static String get _libName {
    if (Platform.isAndroid) return 'libblend_kit_rs.so';
    if (Platform.isIOS) return 'blend_kit_rs.framework/blend_kit_rs';
    if (Platform.isMacOS) return 'libblend_kit_rs.dylib';
    if (Platform.isWindows) return 'blend_kit_rs.dll';
    // 鸿蒙 NEXT 通过 Platform.isLinux 兜底 (Dart 还没出 isOhos).
    return 'libblend_kit_rs.so';
  }

  @override
  Future<BlendResult> solve(BlendRequest req) async {
    final inputJson = jsonEncode(req.toJson());
    final inputPtr = inputJson.toNativeUtf8();
    try {
      final outPtr = _solve(inputPtr);
      try {
        final outJson = outPtr.toDartString();
        final j = jsonDecode(outJson) as Map<String, dynamic>;
        return BlendResult.fromJson(j);
      } finally {
        _free(outPtr);
      }
    } finally {
      malloc.free(inputPtr);
    }
  }
}

// ============================================================
// Dart fallback (无 LP, 仅占位)
// ============================================================

/// 仅算成本 / 检查指标, **不做** LP 优化. 给测试 / Web demo 用.
/// 真机一定要走 FFI.
class DartFallbackBlendSolver implements BlendSolver {
  @override
  Future<BlendResult> solve(BlendRequest req) async {
    if (req.coals.isEmpty) {
      return const BlendResult(
        ok: false,
        reason: 'no coals',
        recipe: {},
        orders: [],
        indicatorCheck: [],
        warnings: ['Dart fallback solver: 没接 Rust LP, 配方为空'],
      );
    }
    // 均分: 每种煤等比例
    final n = req.coals.length;
    final ratio = 1.0 / n;
    final recipe = <String, double>{
      for (final c in req.coals) c.name: ratio,
    };

    final fobPerTon =
        req.coals.fold<double>(0, (sum, c) => sum + c.fob * ratio);
    final frtPerTon =
        req.coals.fold<double>(0, (sum, c) => sum + c.frt * ratio);
    final cifPerTon = fobPerTon + frtPerTon;

    return BlendResult(
      ok: true,
      recipe: recipe,
      cost: CostBreakdown(
        fobPerTon: fobPerTon,
        frtPerTon: frtPerTon,
        cifPerTon: cifPerTon,
        totalFob: req.totalQuantity == null ? null : fobPerTon * req.totalQuantity!,
        totalFrt: req.totalQuantity == null ? null : frtPerTon * req.totalQuantity!,
        totalCif: req.totalQuantity == null ? null : cifPerTon * req.totalQuantity!,
      ),
      orders: req.coals
          .map((c) => OrderItem(
                coal: c.name,
                ratio: ratio,
                tons: req.totalQuantity == null
                    ? null
                    : ratio * req.totalQuantity!,
              ))
          .toList(),
      indicatorCheck: const [],
      warnings: const [
        'Dart fallback solver: 均分模式, 未做 LP 优化, 不验证合同指标',
      ],
    );
  }
}
