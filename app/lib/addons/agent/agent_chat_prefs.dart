// Per-chat choices of this device: the host a chat runs on and the model
// picked for it. Kept in `<data dir>/agent_chats.json` (per device, never
// synced), keyed by chat id. A chat's session lives on its host, so the
// host is written once, when the chat is created, and never changes.
// [AgentChatPrefs] holds the choices; an [AgentChatPrefsStorage] keeps them
// ([AgentChatPrefsFile] is the file).

import 'dart:io';

import '../../services/addon_services.dart';
import 'agent_chat_prefs_file.dart';

export 'agent_chat_prefs_file.dart' show AgentChatPrefsFile;

class AgentChatPref {
  const AgentChatPref({required this.host, this.model, this.compacted = false});

  final AgentHost host;

  /// `provider/model` picked in the chat; null: the default.
  final String? model;

  /// The chat shows its older history as summary cards (Settings: Chat history,
  /// Compacted). Off by default; saved only when on.
  final bool compacted;

  AgentChatPref withModel(String? m) =>
      AgentChatPref(host: host, model: m, compacted: compacted);

  AgentChatPref withCompacted(bool v) =>
      AgentChatPref(host: host, model: model, compacted: v);

  Map<String, dynamic> toJson() => {
    'host': host.name,
    if (model != null) 'model': model,
    if (compacted) 'history': 'compacted',
  };

  static AgentChatPref? fromJson(Object? j) {
    if (j is! Map) return null;
    final host = AgentHost.values.where((h) => h.name == j['host']).firstOrNull;
    if (host == null) return null;
    final model = j['model'];
    return AgentChatPref(
      host: host,
      model: model is String ? model : null,
      compacted: j['history'] == 'compacted',
    );
  }
}

/// Where the per-chat choices are kept. [AgentChatPrefsFile] is the
/// device's `agent_chats.json`.
abstract interface class AgentChatPrefsStorage {
  /// The saved choices; a missing or unreadable store gives none.
  Map<String, AgentChatPref> read();

  /// Replaces the saved choices with [choices]. May throw; [AgentChatPrefs]
  /// keeps its choices in memory either way.
  void write(Map<String, AgentChatPref> choices);
}

class AgentChatPrefs {
  /// Starts from what [storage] holds (null: in memory only).
  AgentChatPrefs({this.storage}) {
    final s = storage;
    if (s != null) _byChat.addAll(s.read());
  }

  /// Where the choices are saved (null: in memory only).
  final AgentChatPrefsStorage? storage;
  final Map<String, AgentChatPref> _byChat = {};

  /// Loads [file] ([AgentChatPrefsFile]); a missing or unreadable file gives
  /// no choices, and null keeps them in memory only.
  static AgentChatPrefs load(File? file) =>
      AgentChatPrefs(storage: file == null ? null : AgentChatPrefsFile(file));

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

  /// Saves whether chat [id] shows compacted history; a chat with no recorded
  /// host is recorded on [host].
  void setCompacted(String id, AgentHost host, bool value) {
    final p = _byChat[id];
    if ((p?.compacted ?? false) == value) return;
    _byChat[id] = (p ?? AgentChatPref(host: host)).withCompacted(value);
    _save();
  }

  void remove(String id) {
    if (_byChat.remove(id) != null) _save();
  }

  /// A failed write is dropped: the choice stays in memory for this run and
  /// the next successful write saves it.
  void _save() {
    try {
      storage?.write(Map.of(_byChat));
    } catch (_) {}
  }
}
