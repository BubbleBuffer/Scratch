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
// - Its MCP servers are the AI settings' list, like the chat engine's:
//   [setMcpServers] applies a change to the running engine at once, without
//   reconnecting (the list is not part of [HostRelayConfig.key]).
// - Settings shows the connection state while it is open ([watchStatus]).
// - Its TLS trusts the certificates this device trusts for the hub
//   ([HostRelayConfig.trusted], like sync). Trusting or revoking one gives
//   the running relay the new list (`hostRelaySetTrust`): its next attempt
//   uses it, and a relay waiting to retry tries again at once. It is not
//   part of [HostRelayConfig.key], so the engine is not reopened.
//
// [AgentStore] owns one link per vault and calls [sync] when settings change.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/debounce.dart';
import '../../core/paths.dart';
import '../../src/rust/api/agent.dart';
import '../../src/rust/api/types.dart' show TrustedHubCertificate;

/// What the relay runs with, or null when it should not run.
class HostRelayConfig {
  const HostRelayConfig({
    required this.url,
    required this.device,
    required this.token,
    required this.open,
    required this.key,
    required this.vaultRoot,
    this.trusted = const [],
    this.trustKey = '',
  });

  final String url, device, token;

  /// The vault directory whose device key proves this device.
  final String vaultRoot;

  /// What this device trusts for the hub beyond the public roots.
  final List<TrustedHubCertificate> trusted;

  /// Changes whenever [trusted] changes.
  final String trustKey;

  /// Opens the relay's engine.
  final Future<NotesAgent> Function() open;

  /// Changes whenever anything above changes (restart when it does).
  final String key;
}

class HostRelayLink {
  HostRelayLink({this.onChanged});

  /// After [mcpError] changes.
  final VoidCallback? onChanged;

  NotesAgent? _agent;
  String _key = '';

  /// The trust the running relay was last given.
  List<TrustedHubCertificate> _trusted = const [];
  String _trustKey = '';

  /// Tests: stands in for the engine's `hostRelaySetTrust`.
  @visibleForTesting
  static Future<void> Function(List<TrustedHubCertificate> trusted)? debugSetTrust;
  Future<void> _work = Future.value();
  final _debounce = Debounce();
  bool _closed = false;

  /// Whether the relay's engine is open and its relay started.
  bool get running => _agent != null;

  /// The last failure to start, if any (shown in the connection state).
  String? error;

  /// Why the relay engine refused the MCP servers (null: applied).
  String? mcpError;

  /// Tests: stands in for the engine's `setMcpServers`.
  @visibleForTesting
  static Future<void> Function(String json)? debugSetMcpServers;

  /// Tests: stands in for the engine's `hostRelayStatus`, running or not.
  @visibleForTesting
  static Future<AgentHostRelayStatus> Function()? debugStatus;

  /// How often Settings reads the connection state while it shows it.
  static const statusInterval = Duration(seconds: 2);

  /// Starts, restarts or stops the relay for [config]. Settings typed per
  /// keystroke settle first.
  void sync(HostRelayConfig? config, {bool now = false}) {
    if (_closed) return;
    final key = config?.key ?? '';
    if (key == _key) {
      _debounce.cancel();
      if (config != null) _setTrust(config);
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
      _trusted = config.trusted;
      _trustKey = config.trustKey;
      await agent.hostRelayStart(
        url: config.url,
        device: config.device,
        token: config.token,
        trusted: config.trusted,
        vaultRoot: config.vaultRoot,
      );
      if (_closed) {
        await agent.hostRelayStop();
        agent.dispose();
        return;
      }
      _agent = agent;
      error = null;
      await _sendMcp(agent);
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

  // ------------------------------- TLS trust

  /// Gives the running relay [config]'s trust when it changed (the user
  /// trusted or revoked a certificate); a relay starting now gets it from
  /// [_apply].
  void _setTrust(HostRelayConfig config) {
    if (config.trustKey == _trustKey) return;
    _trustKey = config.trustKey;
    _trusted = config.trusted;
    _work = _work.then((_) async {
      final agent = _agent;
      if (_closed || (agent == null && debugSetTrust == null)) return;
      final trusted = _trusted;
      try {
        final debug = debugSetTrust;
        await (debug != null ? debug(trusted) : agent!.hostRelaySetTrust(trusted: trusted));
      } catch (e) {
        AppLog.info('host relay trust: $e');
      }
    });
  }

  // ------------------------------- MCP servers

  String _mcp = '{}';

  /// Gives the relay engine [json] (the AI settings' MCP servers, in the
  /// shape `setMcpServers` takes): now when it runs, else when it starts.
  void setMcpServers(String json) {
    if (_closed || json == _mcp) return;
    _mcp = json;
    final agent = _agent;
    if (agent == null) return;
    _work = _work.then((_) => _sendMcp(agent));
  }

  Future<void> _sendMcp(NotesAgent agent) async {
    if (!identical(agent, _agent) || _closed) return;
    final json = _mcp;
    String? failed;
    try {
      final debug = debugSetMcpServers;
      await (debug != null ? debug(json) : agent.setMcpServers(json: json));
    } catch (e) {
      failed = '$e';
      AppLog.info('host relay MCP servers: $e');
    }
    if (failed != mcpError) {
      mcpError = failed;
      onChanged?.call();
    }
  }

  // ------------------------------- connection state

  final _status = ValueNotifier<AgentHostRelayStatus?>(null);

  /// The connection state as last read (null: not read yet). Read every
  /// [statusInterval] while anyone [watchStatus]es.
  ValueListenable<AgentHostRelayStatus?> get status => _status;

  int _watchers = 0;
  Timer? _poll;

  /// Starts reading the connection state (Settings, while it shows it).
  /// Each call needs an [unwatchStatus].
  void watchStatus() {
    if (_closed) return;
    if (_watchers++ > 0) return;
    unawaited(_readStatus());
    _poll = Timer.periodic(statusInterval, (_) => _readStatus());
  }

  void unwatchStatus() {
    if (_watchers == 0) return;
    if (--_watchers > 0) return;
    _poll?.cancel();
    _poll = null;
  }

  Future<void> _readStatus() async {
    final agent = _agent;
    final debug = debugStatus;
    AgentHostRelayStatus next;
    try {
      if (debug != null) {
        next = await debug();
      } else if (agent != null) {
        next = await agent.hostRelayStatus();
      } else if (_key.isEmpty) {
        // Not configured (no https hub yet), or turned off.
        next = const AgentHostRelayStatus(state: 'off');
      } else {
        // Starting, or it failed to start.
        next = AgentHostRelayStatus(
          state: error == null ? 'connecting' : 'off',
          error: error,
        );
      }
    } catch (e) {
      next = AgentHostRelayStatus(state: 'off', error: '$e');
    }
    if (!_closed) _status.value = next;
  }

  /// Stops the relay for good.
  Future<void> close() {
    _closed = true;
    _poll?.cancel();
    _poll = null;
    _watchers = 0;
    _debounce.cancel();
    _key = '';
    return _work = _work.then((_) => _stop());
  }
}
