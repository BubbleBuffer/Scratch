import 'package:flutter/widgets.dart';

import '../core/models.dart';
import 'command_registry.dart';

/// Data and host-owned widgets shared by every presentation of a note.
class NoteArgs {
  const NoteArgs({
    required this.pageId,
    this.pageType = documentType,
    this.title = '',
    this.icon,
    this.editor = const SizedBox.shrink(),
    this.titleController,
    this.onTitleChanged,
    this.onOpen,
    this.target,
    this.instanceId,
    this.onTargetChanged,
    this.onFallbackChanged,
  });

  final String pageId, pageType, title;
  final String? icon;
  final Widget editor;
  final TextEditingController? titleController;
  final ValueChanged<String>? onTitleChanged;
  final VoidCallback? onOpen;

  /// Source-owned embed anchor, independent of the note's body storage.
  final String? target, instanceId;
  final ValueChanged<String>? onTargetChanged, onFallbackChanged;
}

typedef NoteBuilder = Widget Function(BuildContext context, NoteArgs args);

/// An installed note type. Builders leave storage and navigation with the host.
class NoteType {
  const NoteType({
    required this.id,
    required this.label,
    required this.chip,
    required this.card,
    this.boardCard,
    NoteBuilder? embed,
    required this.fullPage,
    this.commands = const [],
    this.usesEditor = true,
    this.claimsPage,
    this.changes,
    this.bodySource = 'vault',
    // Keep the public constructor name while resolving absent embeds to cards.
    // ignore: prefer_initializing_formals
  }) : _embed = embed;

  final String id, label;

  /// The source owning this type's body; external notes need no vault page.
  final String bodySource;
  final NoteBuilder chip, card, fullPage;

  /// Optional compact board presentation. Database boards retain their row
  /// preview and properties when absent; other boards may fall back to [card].
  final NoteBuilder? boardCard;

  /// Types without a larger inline presentation use their card.
  final NoteBuilder? _embed;
  NoteBuilder get embed => _embed ?? card;
  final List<Command> commands;
  final bool usesEditor;

  /// Optimistic type changes before the host's page metadata reloads.
  final bool Function(String pageId)? claimsPage;
  final Listenable? changes;
}

class NoteTypeRegistry extends ChangeNotifier {
  final _types = <String, NoteType>{};

  void register(NoteType type) {
    _types[type.id]?.changes?.removeListener(notifyListeners);
    _types[type.id] = type;
    type.changes?.addListener(notifyListeners);
    notifyListeners();
  }

  NoteType? of(String id) => _types[id];
  Iterable<NoteType> get all => _types.values;

  NoteType? forPage(NoteArgs args) {
    for (final type in _types.values) {
      if (type.claimsPage?.call(args.pageId) ?? false) return type;
    }
    return of(args.pageType);
  }

  /// Unknown types retain the editor's existing markdown fallback.
  Widget buildFullPage(BuildContext context, NoteArgs args) =>
      forPage(args)?.fullPage(context, args) ?? args.editor;

  @override
  void dispose() {
    for (final type in _types.values) {
      type.changes?.removeListener(notifyListeners);
    }
    super.dispose();
  }
}

final noteTypeRegistry = NoteTypeRegistry();

/// Rebuilds when an add-on reports an optimistic type transition.
class NotePageBody extends StatelessWidget {
  const NotePageBody({super.key, required this.args, this.registry});
  final NoteArgs args;
  final NoteTypeRegistry? registry;

  @override
  Widget build(BuildContext context) {
    final types = registry ?? noteTypeRegistry;
    return ListenableBuilder(
      listenable: types,
      builder: (context, _) => types.buildFullPage(context, args),
    );
  }
}
