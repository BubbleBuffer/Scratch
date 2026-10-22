// "Let the hub use this computer" (Settings > Sync, desktop only): while it
// is on and this device is signed in to an https hub, a separate agent engine
// keeps one socket open to the hub (`NotesAgent.hostRelayStart`), and
// ChatGPT or claude.ai on `/mcp/agent?host=<this device>` run this computer's
// agent tools through the hub.
//
// - The relay engine keeps its own store (`<data dir>/host-relay/`), so it
//   never contends with the chat engine for its store, and relay sessions
//   stay out of the chat list.
// - Its tools follow the AI settings like the chat engine's (desktop tools
//   and their root), with no approval prompts.
// - Turning it off, signing out or closing the vault stops it: the socket
//   closes and running calls are cancelled.
//
// [AgentStore] owns one link per vault and calls [sync] when settings change.

import 'dart:async';

import '../../core/debounce.dart';
import '../../core/paths.dart';
import '../../src/rust/api/agent.dart';

/// What the relay runs with, or null when it should not run.
class HostRelayConfig {
  const HostRelayConfig({
    required this.url,
    required this.device,
    required this.token,
    required this.open,
    required this.key,
  });

  final String url, device, token;

  /// Opens the relay's engine.
  final Future<NotesAgent> Function() open;

  /// Changes whenever anything above changes (restart when it does).
  final String key;
}

class HostRelayLink {
  NotesAgent? _agent;
  String _key = '';
  Future<void> _work = Future.value();
  final _debounce = Debounce();
  bool _closed = false;

  /// Whether the relay's engine is open and its relay started.
  bool get running => _agent != null;

  /// The last failure to start, if any (logged, not shown).
  String? error;

  /// Starts, restarts or stops the relay for [config]. Settings typed per
  /// keystroke settle first.
  void sync(HostRelayConfig? config, {bool now = false}) {
    if (_closed) return;
    final key = config?.key ?? '';
    if (key == _key) {
      _debounce.cancel();
      return;
    }
    void apply() {
      if (_closed || key == _key) return;
      _key = key;
      _work = _work.then((_) => _apply(config)).catchError((Object e) {
        AppLog.info('host relay: $e');
      });
    }

    if (now || config == null) {
      _debounce.cancel();
      apply();
    } else {
      _debounce.run(const Duration(milliseconds: 600), apply);
    }
  }

  /// Waits for pending starts and stops (tests).
  Future<void> settled() => _work;

  Future<void> _apply(HostRelayConfig? config) async {
    await _stop();
    if (config == null || _closed) return;
    NotesAgent? agent;
    try {
      agent = await config.open();
      await agent.hostRelayStart(
        url: config.url,
        device: config.device,
        token: config.token,
      );
      if (_closed) {
        await agent.hostRelayStop();
        agent.dispose();
        return;
      }
      _agent = agent;
      error = null;
    } catch (e) {
      agent?.dispose();
      error = '$e';
      AppLog.info('host relay failed to start: $e');
    }
  }

  Future<void> _stop() async {
    final agent = _agent;
    _agent = null;
    if (agent == null) return;
    try {
      await agent.hostRelayStop();
    } catch (e) {
      AppLog.info('host relay failed to stop: $e');
    }
    agent.dispose();
  }

  /// Stops the relay for good.
  Future<void> close() {
    _closed = true;
    _debounce.cancel();
    _key = '';
    return _work = _work.then((_) => _stop());
  }
}
