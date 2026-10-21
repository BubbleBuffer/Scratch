// Sends a page's edits to `vault.apply` one batch at a time, in order.
// Consecutive SetText ops for the same block are coalesced while a batch is
// in flight, so fast typing never queues up.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../core/models.dart';
import '../core/vault.dart';

class OpQueue {
  OpQueue(this.vault, this.pageId);
  final Vault vault;
  final String pageId;
  final List<BOp> _pending = [];
  bool _busy = false;
  Completer<void>? _idle;
  void Function(String error)? onError;

  /// Bumped on every [add] (lets a reader tell whether edits happened while
  /// it was waiting).
  int generation = 0;

  bool get idle => !_busy && _pending.isEmpty;

  void add(List<BOp> ops) {
    if (ops.isEmpty) return;
    generation++;
    for (final op in ops) {
      if (op.op == OpKind.setText && _pending.isNotEmpty) {
        final last = _pending.last;
        if (last.op == OpKind.setText && last.id == op.id) {
          _pending[_pending.length - 1] = op;
          continue;
        }
      }
      _pending.add(op);
    }
    _pump();
  }

  Future<void> _pump() async {
    if (_busy || _pending.isEmpty) return;
    _busy = true;
    while (_pending.isNotEmpty) {
      final batch = List<BOp>.of(_pending);
      _pending.clear();
      try {
        await vault.apply(pageId, batch);
      } catch (e) {
        debugPrint('apply failed: $e');
        onError?.call('$e');
      }
    }
    _busy = false;
    _idle?.complete();
    _idle = null;
  }

  /// Completes when everything queued so far has been applied.
  Future<void> drain() {
    if (idle) return Future.value();
    return (_idle ??= Completer<void>()).future;
  }
}
