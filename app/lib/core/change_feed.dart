// The change feed: the single source of refresh. It polls the core's
// `poll_changes` on a timer and right after local writes (Vault.onWrite),
// and hands each non-empty batch to its listeners. Local edits, sync and
// import all arrive the same way, so stores never need to know where a
// change came from.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../src/rust/api/types.dart';
import 'vault.dart';
import 'debounce.dart';

typedef ChangeListener = void Function(ChangeBatch batch);

class ChangeFeed {
  ChangeFeed(this.vault, {this.interval = const Duration(seconds: 1), this.wakeDelay = const Duration(milliseconds: 30)}) {
    vault.addCloseListener(stop);
    if (!vault.isClosed) vault.onWrite = wake;
  }

  final Vault vault;

  /// Poll period when nothing wakes the feed (catches sync and other
  /// writers outside this isolate).
  final Duration interval;

  /// Delay between a local write and the poll it triggers (writes in one
  /// burst share a poll).
  final Duration wakeDelay;

  final _listeners = <ChangeListener>[];
  int _seq = 0;
  Timer? _timer;
  Timer? _wake;
  bool _polling = false;
  Completer<void>? _pollDone;
  bool _again = false;
  bool _stopped = true;

  /// The last sequence number seen.
  int get seq => _seq;

  /// No poll is scheduled or running (besides the periodic one).
  bool get idle => _wake == null && !_polling;

  void addListener(ChangeListener l) => _listeners.add(l);
  void removeListener(ChangeListener l) => _listeners.remove(l);

  /// Starts polling from the current state (earlier changes are skipped).
  Future<void> start() async {
    if (vault.isClosed) return;
    _stopped = false;
    vault.onWrite = wake;
    final b = await vault.pollChanges(0);
    if (_stopped || vault.isClosed) return;
    _seq = b.seq;
    // Harnesses poll explicitly at each settle boundary; a zero-period
    // timer would spin and prevent the feed from becoming idle.
    if (!Debounce.instant) _timer = Timer.periodic(interval, (_) => poll());
  }

  void stop() {
    _stopped = true;
    _timer?.cancel();
    _timer = null;
    _wake?.cancel();
    _wake = null;
    if (vault.onWrite == wake) vault.onWrite = null;
  }

  /// Polls soon (after [wakeDelay]).
  void wake() {
    if (_stopped || vault.isClosed || _wake != null) return;
    _wake = Timer(Debounce.testDelay(wakeDelay), () {
      _wake = null;
      poll();
    });
  }

  /// Polls now and delivers what changed.
  Future<void> poll() async {
    if (_stopped || vault.isClosed) return;
    if (_polling) {
      _again = true;
      await _pollDone!.future;
      return;
    }
    _polling = true;
    final done = _pollDone = Completer<void>();
    try {
      do {
        _again = false;
        final b = await vault.pollChanges(_seq).then((b) => ChangeBatch(seq: b.seq, reset: b.reset, changes: b.changes.where((c) => c.kind != ChangeKind.indexCommitted).toList()));
        if (_stopped) return;
        _seq = b.seq;
        if (b.reset || b.changes.isNotEmpty) {
          for (final l in List.of(_listeners)) {
            if (_stopped || vault.isClosed) return;
            try {
              l(b);
            } catch (e, st) {
              debugPrint('change listener failed: $e\n$st');
            }
          }
        }
      } while (_again && !_stopped);
    } finally {
      _polling = false;
      _pollDone = null;
      done.complete();
    }
  }
}

/// Helpers to read a batch.
extension ChangeBatchX on ChangeBatch {
  bool get treeChanged =>
      reset || changes.any((c) => c.kind != ChangeKind.pageChanged);

  /// Page ids changed in place (content, title, icon or properties).
  Iterable<String> get changedPages =>
      changes.where((c) => c.kind == ChangeKind.pageChanged && c.page != null).map((c) => c.page!);

  bool touches(String pageId) => reset || changes.any((c) => c.page == pageId);
}
