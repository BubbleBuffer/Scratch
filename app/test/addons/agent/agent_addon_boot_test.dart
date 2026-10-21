// AgentAddon alone, without native initialization: its registrations land in
// its own context, the process-wide registries are untouched, and every
// surface it declares is present (block with codecs, commands, slash item,
// settings sections, shell panel and the chat note type).

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/registry/note_type_registry.dart';

void main() {
  test('AgentAddon has its stable id and no dependencies', () {
    const addon = AgentAddon();
    expect(addon.id, 'agent');
    expect(addon.dependsOn, isEmpty);
  });

  test('AgentAddon alone registers its surfaces and leaves the global registries alone', () {
    final globalBlocks = blockRegistry.all.toList();
    final globalTypes = noteTypeRegistry.all.toList();
    final globalCommands = commandRegistry.all.toList();
    final ctx = AddonContext();
    final booted = bootAddons([const AgentAddon()], ctx);

    expect(booted.map((a) => a.id), ['agent']);
    expect(ctx.installedIds, {'agent'});
    expect(blockRegistry.all, globalBlocks);
    expect(noteTypeRegistry.all, globalTypes);
    expect(commandRegistry.all, globalCommands);

    // Block: the chat reference kind, in the `/` menu, with all codecs.
    expect(ctx.blockKinds.has(Kinds.chatRef), isTrue);
    final block = ctx.blockKinds.of(Kinds.chatRef);
    expect(block.inMenu, isTrue);
    expect(block.insertOnly, isTrue);
    expect(block.codecs, isNotNull);
    expect(block.body, isNotNull);
    expect(ctx.blockKinds.incompleteCodecKinds, isEmpty);

    // Slash item: the menu entry that inserts the chat reference block.
    final slash = ctx.slashItems.of('chat_ref');
    expect(slash, isNotNull);
    expect(slash!.blockKind, Kinds.chatRef);
    expect(slash.label, 'Chat');

    // Commands: the agent's open command is always registered.
    expect(ctx.commands.of('agent.open'), isNotNull);

    // Settings: desktop and mobile sections both sit in the AI page.
    final desktop = ctx.settingsSections.of('agent.desktop');
    final mobile = ctx.settingsSections.of('agent.mobile');
    expect(desktop, isNotNull);
    expect(mobile, isNotNull);
    expect(desktop!.location, 'ai');
    expect(mobile!.location, 'ai');

    // Shell panel: the agent dock, opened on its route name.
    final panel = ctx.shellPanels.of('agent');
    expect(panel, isNotNull);
    expect(panel!.routeName, agentRouteName);
    expect(agentRouteName, '/agent');

    // Note type: the chat type reads its body from the agent store.
    final chat = ctx.noteTypes.of('chat');
    expect(chat, isNotNull);
    expect(identical(chat, chatNoteType), isTrue);
    expect(chat!.label, 'Chat');
    expect(chat.bodySource, 'agent');
    expect(chat.usesEditor, isFalse);

    ctx.noteTypes.dispose();
  });
}
