// Per-chat choices of this device: the host a chat runs on and the model
// picked for it. Kept in `<data dir>/agent_chats.json` (per device, never
// synced), keyed by chat id. A chat's session lives on its host, so the
// host is written once, when the chat is created, and never changes.

import 'dart:convert';
import 'dart:io';

import '../../services/addon_services.dart';

class AgentChatPref {
  const AgentChatPref({required this.host, this.model});

  final AgentHost host;

  /// `provider/model` picked in the chat; null: the default.
  final String? model;

  AgentChatPref withModel(String? m) => AgentChatPref(host: host, model: m);

  Map<String, dynamic> toJson() => {
    'host': host.name,
    if (model != null) 'model': model,
  };

  static AgentChatPref? fromJson(Object? j) {
    if (j is! Map) return null;
    final host = AgentHost.values.where((h) => h.name == j['host']).firstOrNull;
    if (host == null) return null;
    final model = j['model'];
    return AgentChatPref(host: host, model: model is String ? model : null);
  }
}

class AgentChatPrefs {
  AgentChatPrefs({this.file});

  /// Where the choices are saved (null: in memory only).
  final File? file;
  final Map<String, AgentChatPref> _byChat = {};

  /// Loads [file]; a missing or unreadable file gives no choices.
  static AgentChatPrefs load(File? file) {
    final p = AgentChatPrefs(file: file);
    try {
      if (file != null && file.existsSync()) {
        final j = jsonDecode(file.readAsStringSync());
        if (j is Map) {
          for (final e in j.entries) {
            final pref = AgentChatPref.fromJson(e.value);
            if (e.key is String && pref != null) p._byChat[e.key] = pref;
          }
        }
      }
    } catch (_) {}
    return p;
  }

  AgentChatPref? operator [](String id) => _byChat[id];

  /// Records that chat [id] runs on [host] (with an optional first model).
  void created(String id, AgentHost host, {String? model}) {
    _byChat[id] = AgentChatPref(host: host, model: model);
    _save();
  }

  /// Saves the model picked in chat [id]; a chat with no recorded host is
  /// recorded on [host].
  void pickModel(String id, AgentHost host, String model) {
    final p = _byChat[id] ?? AgentChatPref(host: host);
    _byChat[id] = p.withModel(model);
    _save();
  }

  void remove(String id) {
    if (_byChat.remove(id) != null) _save();
  }

  void _save() {
    final f = file;
    if (f == null) return;
    try {
      f.parent.createSync(recursive: true);
      final tmp = File('${f.path}.tmp');
      tmp.writeAsStringSync(
        const JsonEncoder.withIndent('  ').convert({
          for (final e in _byChat.entries) e.key: e.value.toJson(),
        }),
        flush: true,
      );
      tmp.renameSync(f.path);
    } catch (_) {}
  }
}
