import 'dart:async';

import 'package:flutter/widgets.dart';

import '../core/change_feed.dart';
import '../core/models.dart';
import '../registry/addon_registry.dart';
import '../registry/embed_block.dart' show rememberEmbedTitle;
import '../registry/note_type_registry.dart';
import '../state/app_state.dart';
import 'embed_document.dart';
import 'note_preview.dart';
import 'ui.dart';

/// Storage can retain arbitrary nesting; this bounds only inline drawing.
const maxEmbedDepth = 4;

/// The active note ancestry follows the widget tree, including through type
/// builders. The first embed seeds it with its block host's page identity.
class EmbedScope extends InheritedWidget {
  const EmbedScope({
    super.key,
    required this.ancestors,
    this.loader,
    required super.child,
  });
  final List<String> ancestors;
  final EmbedNoteLoader? loader;
  static EmbedScope? of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<EmbedScope>();
  @override
  bool updateShouldNotify(EmbedScope oldWidget) =>
      oldWidget.ancestors != ancestors || oldWidget.loader != loader;
}

class EmbedNoteData {
  const EmbedNoteData({
    required this.id,
    required this.title,
    required this.pageType,
    this.icon,
    this.page,
    this.hit,
  });
  factory EmbedNoteData.fromPage(PageData page) => EmbedNoteData(
    id: page.id,
    title: page.title,
    pageType: page.pageType,
    icon: page.icon,
    page: page,
  );
  factory EmbedNoteData.fromHit(Hit hit) => EmbedNoteData(
    id: hit.id,
    title: hit.title,
    pageType: hit.pageType,
    hit: hit,
  );
  final String id, title, pageType;
  final String? icon;
  final PageData? page;
  final Hit? hit;
}

typedef EmbedNoteLoader = Future<EmbedNoteData?> Function(String targetId);

/// Resolves current vault/index metadata without knowing the target add-on.
/// All presentations go through the same NoteType registry as full pages.
class EmbedNote extends StatefulWidget {
  const EmbedNote({
    super.key,
    required this.targetId,
    required this.rootPageId,
    this.instanceId,
    this.addons,
    this.loader,
    this.onOpenPage,
  });
  final String targetId, rootPageId;
  final String? instanceId;
  final AddonContext? addons;
  final EmbedNoteLoader? loader;
  final ValueChanged<String>? onOpenPage;
  @override
  State<EmbedNote> createState() => _EmbedNoteState();
}

class _EmbedNoteState extends State<EmbedNote> {
  AppState? _app;
  Future<EmbedNoteData?>? _data;
  EmbedNoteLoader? _loader;
  AddonContext? get _addons => widget.addons ?? activeAddonContext;
  NoteTypeRegistry get _types => _addons?.noteTypes ?? noteTypeRegistry;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.maybeOf(context);
    final loader = widget.loader ?? EmbedScope.of(context)?.loader;
    if (_app != app || _loader != loader || _data == null) {
      _app?.feed.removeListener(_changed);
      _app = app;
      _loader = loader;
      _app?.feed.addListener(_changed);
      _data = _load();
    }
  }

  @override
  void didUpdateWidget(EmbedNote oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.targetId != widget.targetId ||
        oldWidget.loader != widget.loader) {
      _loader = widget.loader ?? EmbedScope.of(context)?.loader;
      _data = _load();
    }
  }

  void _changed(ChangeBatch batch) {
    if (!mounted || !batch.touches(widget.targetId)) return;
    final data = _load();
    setState(() {
      _data = data;
    });
  }

  Future<EmbedNoteData?> _load() async {
    if (_loader != null) return _loader!(widget.targetId);
    final vault = _app?.vault;
    if (vault == null) return null;
    try {
      final data = EmbedNoteData.fromPage(
        await vault.loadPage(widget.targetId),
      );
      rememberEmbedTitle(vault, data.id, data.title);
      return data;
    } catch (_) {
      // An external note has index metadata but no page CRDT. Lookup includes
      // disabled types, whose fallback must remain visible on this device.
      try {
        final hit = await vault.run(
          (v) => v.findIndexedNote(id: widget.targetId),
        );
        if (hit == null) return null;
        rememberEmbedTitle(vault, hit.id, hit.title);
        return EmbedNoteData.fromHit(hit);
      } catch (_) {
        return null;
      }
    }
  }

  @override
  void dispose() {
    _app?.feed.removeListener(_changed);
    super.dispose();
  }

  void _open(EmbedNoteData data) {
    final hit = data.hit;
    final source = hit == null ? null : _addons?.searchSources.of(hit.source);
    if (source != null) {
      unawaited(source.open(context, AddonSearchResult.fromHit(hit!)));
    } else if (hit == null) {
      widget.onOpenPage?.call(data.id);
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _types,
    builder: (context, _) => FutureBuilder<EmbedNoteData?>(
      future: _data,
      builder: (context, snapshot) {
        final data = snapshot.data;
        if (data == null || data.id != widget.targetId) {
          return Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 8),
            child: Text(
              snapshot.connectionState == ConnectionState.waiting
                  ? 'Loading note…'
                  : 'Embedded note unavailable',
              style: ts(context.type.ui, context.colors.textTertiary),
            ),
          );
        }
        final ancestors =
            EmbedScope.of(context)?.ancestors ?? [widget.rootPageId];
        final args = NoteArgs(
          pageId: data.id,
          pageType: data.pageType,
          title: data.title,
          icon: data.icon,
          instanceId: widget.instanceId,
          onOpen: () => _open(data),
          editor: data.page == null
              ? Text(
                  data.hit?.snippet ?? '',
                  style: ts(context.type.body, context.colors.textPrimary),
                )
              : EmbedDocument(
                  page: data.page!,
                  registry: _addons?.blockKinds,
                  onOpenPage: widget.onOpenPage,
                ),
        );
        final type = _types.forPage(args);
        if (ancestors.contains(data.id) || ancestors.length > maxEmbedDepth) {
          return KeyedSubtree(
            key: ValueKey('embed-link-${widget.instanceId ?? data.id}'),
            child: (type?.chip ?? noteChip)(context, args),
          );
        }
        return EmbedScope(
          ancestors: [...ancestors, data.id],
          loader: _loader,
          child: Builder(
            builder: (context) => type == null
                ? _unknown(context, args)
                : type.embed(context, args),
          ),
        );
      },
    ),
  );

  Widget _unknown(BuildContext context, NoteArgs args) => Container(
    padding: const EdgeInsets.all(12),
    decoration: BoxDecoration(
      color: context.colors.bgSecondary,
      borderRadius: BorderRadius.circular(Radii.sm),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        noteChip(context, args),
        const SizedBox(height: 6),
        Text(
          'Unsupported note type (${args.pageType})',
          style: ts(context.type.uiSmall, context.colors.textTertiary),
        ),
        const SizedBox(height: 8),
        args.editor,
      ],
    ),
  );
}
