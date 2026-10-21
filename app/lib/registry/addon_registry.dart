import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart' show Node;
import '../ui/icons.dart';
import '../services/addon_services.dart';
import 'block_registry.dart';
import 'command_registry.dart';
import 'embed_block.dart';
import 'note_type_registry.dart';
import 'property_registry.dart';
import 'view_registry.dart';

export 'note_drag_payload.dart';

/// An inline mark an add-on draws and opens: the text of a range whose mark
/// has [kind] shows as a chip, and a tap on the chip opens the add-on's panel.
/// The editor owns the state. It calls [watch] once for each distinct payload
/// in a block, draws [label] (null while unknown: the covered text shows) and
/// disposes the state when the mark or the block goes away.
abstract interface class AddonInlineMark {
  String get id;
  BMarkKind get kind;

  /// The live state for [payload] (the mark's text). The editor disposes it.
  ChangeNotifier watch(BuildContext context, String payload);

  /// The chip's text for [state], or null while it is not known yet.
  String? label(ChangeNotifier state);

  /// Opens the chip's panel, anchored to [anchor] (global coordinates).
  void open(BuildContext context, Rect anchor, InlineMarkRef ref);
}

/// What the editor hands an [AddonInlineMark] when its chip is opened: the
/// payload, the state from [AddonInlineMark.watch], the paragraph the chip
/// sits in and the range it covers.
class InlineMarkRef {
  const InlineMarkRef({
    required this.payload,
    required this.state,
    required this.host,
    required this.block,
    required this.range,
  });
  final String payload;
  final ChangeNotifier state;
  final BlockHost host;
  final Node block;
  final TextRange range;
}

/// An add-on's optional contribution immediately after a note's title.
class AddonTitleChip {
  const AddonTitleChip({required this.id, required this.build});
  final String id;
  final Widget? Function(BuildContext context, PageData page) build;
}

/// Providers run in registration order and may omit a chip for this page.
class NoteTitleChips extends StatelessWidget {
  const NoteTitleChips({
    super.key,
    required this.page,
    required this.providers,
  });
  final PageData page;
  final ExtensionRegistry<AddonTitleChip> providers;

  @override
  Widget build(BuildContext context) {
    final chips = <Widget>[];
    for (final provider in providers.all) {
      final chip = provider.build(context, page);
      if (chip != null) chips.add(chip);
    }
    if (chips.isEmpty) return const SizedBox.shrink();
    return Wrap(spacing: 8, runSpacing: 6, children: chips);
  }
}

abstract class Addon {
  const Addon();
  String get id;
  List<String> get dependsOn => const [];
  void register(AddonContext ctx);
}

/// Small keyed registry for extension points without a built-in consumer yet.
class ExtensionRegistry<T> {
  ExtensionRegistry(this.idOf);
  final String Function(T) idOf;
  final _items = <String, T>{};
  void register(T item) => _items[idOf(item)] = item;
  T? of(String id) => _items[id];
  Iterable<T> get all => _items.values;
}

class SlashItem {
  const SlashItem({
    required this.id,
    required this.label,
    required this.run,
    this.description = '',
    this.group = 'Basic blocks',
    this.blockKind,
  });
  final String id, label, description, group;

  /// A block-backed item uses the registered kind's normal insertion path.
  final String? blockKind;
  final void Function(BuildContext context) run;
}

class AddonSettingsSection {
  const AddonSettingsSection({
    required this.id,
    required this.label,
    required this.build,
    this.location = 'addons',
    this.placement = '',
    this.isVisible,
  });
  final String id, label;
  final String location, placement;
  final bool Function(BuildContext context)? isVisible;
  final WidgetBuilder build;
}

/// A source can supply results without exposing its storage to the shell.
abstract class AddonSearchSource {
  const AddonSearchSource();
  String get id;
  String get label;
  String get groupId => id;
  Set<String> get noteTypes => const {};
  String get emptyTitle => 'Untitled';
  Ico get icon => Ico.page;

  /// Indexed sources share the host's ranking, filters and persistent index.
  bool get indexed => false;
  Future<List<AddonSearchResult>> search(String query) async => const [];
  Future<void> open(BuildContext context, AddonSearchResult result);
}

class AddonSearchResult {
  const AddonSearchResult({
    required this.id,
    required this.title,
    this.description = '',
    this.pageType = '',
    this.target,
    this.fuzzy = false,
    this.corrected,
  });
  final String id, title, description;
  final String pageType;

  /// Opaque source-owned navigation anchor; the host never decodes it.
  final String? target;
  final bool fuzzy;
  final String? corrected;

  factory AddonSearchResult.fromHit(Hit hit) => AddonSearchResult(
    id: hit.id,
    title: hit.title,
    description: hit.snippet,
    pageType: hit.pageType,
    target: hit.target,
    fuzzy: hit.fuzzy,
    corrected: hit.corrected,
  );

  Hit toHit(String source) => Hit(
    id: id,
    title: title,
    snippet: description,
    source: source,
    pageType: pageType,
    target: target,
    fuzzy: fuzzy,
    corrected: corrected,
  );
}

/// An optional shell surface. Builders execute only when the host draws it.
class AddonShellPanel {
  const AddonShellPanel({
    required this.id,
    required this.label,
    required this.routeName,
    required this.isEnabled,
    required this.build,
    required this.wrap,
    required this.headerAction,
  });
  final String id, label, routeName;
  final bool Function(BuildContext context) isEnabled;
  final WidgetBuilder build, headerAction;
  final Widget Function(BuildContext context, Widget child) wrap;
}

abstract interface class AddonAgentTool {
  String get id;
  String get description;
  Map<String, Object?> get schema;
  Future<Object?> call(Map<String, Object?> arguments);
}

/// A top-level destination supplied by an installed add-on.
class AddonShellNavigation {
  const AddonShellNavigation({
    required this.id,
    required this.label,
    required this.icon,
    required this.routeName,
    required this.open,
    this.order = 0,
    this.sidebar = true,
    this.mobileDock = false,
  });
  final String id, label, routeName;
  final Ico icon;
  final int order;
  final bool sidebar, mobileDock;
  final void Function(BuildContext context) open;
}

/// The floating create action for one or more shell destinations.
class AddonShellPrimaryAction {
  const AddonShellPrimaryAction({
    required this.id,
    required this.label,
    required this.destinations,
    required this.run,
  });
  final String id, label;
  final Set<String> destinations;
  final void Function(BuildContext context) run;
}

/// A fresh context is isolated: registering one add-on never fills globals.
class AddonContext {
  AddonContext({
    NoteTypeRegistry? noteTypes,
    BlockRegistry? blockKinds,
    ViewRegistry? views,
    PropertyRegistry? properties,
    CommandRegistry? commands,
    this.services = const AddonServices(),
  }) : noteTypes = noteTypes ?? NoteTypeRegistry(),
       blockKinds = blockKinds ?? BlockRegistry(),
       views = views ?? ViewRegistry(),
       properties = properties ?? PropertyRegistry(),
       commands = commands ?? CommandRegistry();

  factory AddonContext.global() => AddonContext(
    noteTypes: noteTypeRegistry,
    blockKinds: blockRegistry,
    views: viewRegistry,
    properties: propertyRegistry,
    commands: commandRegistry,
  );

  final NoteTypeRegistry noteTypes;
  final BlockRegistry blockKinds;
  final ViewRegistry views;
  final PropertyRegistry properties;
  final CommandRegistry commands;
  final AddonServices services;
  final slashItems = ExtensionRegistry<SlashItem>((item) => item.id);
  final settingsSections = ExtensionRegistry<AddonSettingsSection>(
    (item) => item.id,
  );
  final searchSources = ExtensionRegistry<AddonSearchSource>((item) => item.id);
  final agentTools = ExtensionRegistry<AddonAgentTool>((item) => item.id);
  final shellPanels = ExtensionRegistry<AddonShellPanel>((item) => item.id);
  final shellNavigation = ExtensionRegistry<AddonShellNavigation>(
    (item) => item.id,
  );
  final shellPrimaryActions = ExtensionRegistry<AddonShellPrimaryAction>(
    (item) => item.id,
  );
  final titleChips = ExtensionRegistry<AddonTitleChip>((item) => item.id);
  final inlineMarks = ExtensionRegistry<AddonInlineMark>((item) => item.id);
  final installedIds = <String>{};

  void registerNoteType(NoteType type) {
    noteTypes.register(type);
    for (final command in type.commands) {
      commands.register(command);
    }
  }
}

/// Stable topological ordering; validate the complete graph before registering.
List<Addon> orderAddons(Iterable<Addon> addons) {
  final byId = <String, Addon>{};
  for (final addon in addons) {
    if (byId.containsKey(addon.id)) {
      throw StateError('Duplicate add-on: ${addon.id}');
    }
    byId[addon.id] = addon;
  }
  final visiting = <String>{}, visited = <String>{};
  final result = <Addon>[];
  void visit(String id, List<String> path) {
    final addon = byId[id];
    if (addon == null) {
      throw StateError(
        'Missing add-on dependency: ${[...path, id].join(' -> ')}',
      );
    }
    if (visited.contains(id)) return;
    if (!visiting.add(id)) {
      throw StateError(
        'Add-on dependency cycle: ${[...path, id].join(' -> ')}',
      );
    }
    for (final dependency in addon.dependsOn) {
      visit(dependency, [...path, id]);
    }
    visiting.remove(id);
    visited.add(id);
    result.add(addon);
  }

  for (final id in byId.keys) {
    visit(id, const []);
  }
  return result;
}

List<Addon> bootAddons(Iterable<Addon> addons, AddonContext ctx) {
  final ordered = orderAddons(addons);
  for (final addon in ordered) {
    addon.register(ctx);
    ctx.installedIds.add(addon.id);
  }
  // Kit blocks do not belong to any add-on. Register after feature blocks so
  // their existing menu ordering stays stable, after validating the graph.
  registerEmbedBlock(ctx);
  return ordered;
}

/// Only the shell assigns the context booted for this process. Isolated boot
/// tests and dependency validation never change runtime availability.
AddonContext? activeAddonContext;
