// The file behind the per-chat choices: `<data dir>/agent_chats.json`, per
// device and never synced. A JSON object keyed by chat id; each value is
// `{"host": "<AgentHost.name>", "model": "<provider/model>"}`, with "model"
// left out when the chat uses the default. Written to `agent_chats.json.tmp`
// and renamed over the file, so a failed write leaves the old file as it was.

import 'dart:convert';
import 'dart:io';

import 'agent_chat_prefs.dart';

class AgentChatPrefsFile implements AgentChatPrefsStorage {
  const AgentChatPrefsFile(this.file);

  final File file;

  /// A missing or unreadable file, or one that is not a JSON object, gives
  /// no choices; entries that are not a valid choice are skipped.
  @override
  Map<String, AgentChatPref> read() {
    final out = <String, AgentChatPref>{};
    try {
      if (file.existsSync()) {
        final j = jsonDecode(file.readAsStringSync());
        if (j is Map) {
          for (final e in j.entries) {
            final pref = AgentChatPref.fromJson(e.value);
            if (e.key is String && pref != null) out[e.key] = pref;
          }
        }
      }
    } catch (_) {}
    return out;
  }

  @override
  void write(Map<String, AgentChatPref> choices) {
    file.parent.createSync(recursive: true);
    final tmp = File('${file.path}.tmp');
    tmp.writeAsStringSync(
      const JsonEncoder.withIndent('  ').convert({
        for (final e in choices.entries) e.key: e.value.toJson(),
      }),
      flush: true,
    );
    tmp.renameSync(file.path);
  }
}
