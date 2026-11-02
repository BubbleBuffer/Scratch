// [KitAgentCommands] over `AgentStore`: the kit backend's command side.

import 'package:agent_kit_core/agent_kit_core.dart' as kit show KitChoice;

import '../../src/rust/api/agent.dart';
import 'agent_store.dart';
import 'kit_backend.dart';

class StoreKitCommands implements KitAgentCommands {
  StoreKitCommands(this.store);
  final AgentStore store;

  @override
  String? get error => store.error;

  @override
  Future<AgentSession?> createSession({String title = 'New chat'}) =>
      store.createSession(title: title);

  @override
  Future<AgentSent?> send(
    String text, {
    AgentMessageContext? context,
    String? sessionId,
  }) => store.send(text, context: context, sessionId: sessionId);

  @override
  Future<void> stop({String? sessionId}) => store.stop(sessionId: sessionId);

  @override
  Future<void> decide(String approvalId, String decision, {String? reason}) =>
      store.decide(approvalId, decision, reason: reason);

  @override
  Future<AgentEdit> undoEdit(String editId) => store.undoEdit(editId);

  @override
  Future<void> renameSession(String id, String title) =>
      store.renameSession(id, title);

  @override
  List<kit.KitChoice> modelChoices(String chatId) => [
    for (final m in store.shownModels(
      current: chatId.isEmpty ? null : store.modelOf(chatId),
    ))
      kit.KitChoice(
        id: m,
        label: store.modelLabel(m),
        detail: store.modelDetail(m),
      ),
  ];

  @override
  String? modelOf(String chatId) => store.modelOf(chatId);

  @override
  void pickModel(String chatId, String model) => store.pickModel(chatId, model);
}
