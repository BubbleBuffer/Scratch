// What Settings > AI > Agent offers for each runner, without widgets: the
// chosen runner, the help line for each runner and tool set, and the model
// pickers' options and values. The choices themselves are saved by
// [AgentStore] (vault defaults, synced); agent_runner_settings.dart lays
// them out.

import '../../services/addon_services.dart';
import 'agent_store.dart';

/// What each runner does, in one line.
String runnerHelp(AgentHost h) => switch (h) {
  AgentHost.hub => 'Your hub runs the AI; works from every device.',
  AgentHost.local => 'This computer runs the AI with its own models and tools.',
};

/// The runner chosen for the vault (synced), or the built-in default.
AgentHost chosenRunner(AgentStore store) =>
    store.vaultDefaults.host ?? builtInAgentHost;

/// The hub model picker's options: the hub default (null), the models the
/// hub lists ([ids]) and the [stored] choice when the hub no longer lists it.
List<String?> hubModelOptions(List<String> ids, String? stored) => [
  null,
  ...ids,
  if (stored != null && !ids.contains(stored)) stored,
];

/// The hub model picker's label for [id]: the model's name ([modelLabel]),
/// or for the hub default (null) the model the hub uses by default, when
/// known.
String hubModelLabel(
  String? id,
  String? hostDefault,
  String Function(String id) modelLabel,
) => id == null
    ? hostDefault == null
          ? 'Hub default'
          : 'Hub default (${modelLabel(hostDefault)})'
    : modelLabel(id);

/// The computer model picker's value: the [stored] model when this computer
/// still lists it, else the first one listed. [ids] is not empty.
String computerModelValue(List<String> ids, String? stored) =>
    stored != null && ids.contains(stored) ? stored : ids.first;

/// What tool set [set] gives on [runner], in one line.
String toolSetHelp(String set, AgentHost runner) {
  final onHub = runner == AgentHost.hub;
  final what = switch (set) {
    'read' => 'Reads the vault and changes nothing.',
    'write' => 'Reads and edits the vault.',
    _ =>
      onHub
          ? 'Every tool the hub has.'
          : 'The vault, plus files, commands and MCP servers here.',
  };
  return onHub
      ? '$what Jobs use it at once; hub chats after the hub restarts (an update or reboot).'
      : what;
}
