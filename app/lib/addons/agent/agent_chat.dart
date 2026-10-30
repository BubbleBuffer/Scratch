import '../../services/addon_services.dart';

import 'package:agent_kit/agent_kit.dart'
    show
        ApprovalBar,
        ApprovalKeyPane,
        ItemRow,
        OutlineAction,
        OutlineControllerContents,
        OutlineChapter,
        OutlineList,
        OutlineSection,
        OutlineWhereStrip,
        TimelineEntry,
        TranscriptAnchor,
        TranscriptList,
        outlineSegments,
        outlineStripColors;
import 'package:agent_kit_core/agent_kit_core.dart' show ApprovalChoice;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'add_to_note.dart';
import 'agent_composer.dart';
import 'agent_history.dart';
import 'agent_history_setting.dart';
import 'agent_outline.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'chat_reader_controller.dart';
import 'chat_reference.dart';
import 'chat_transcript_view.dart';
import 'kit_backend.dart' show kitApproval;
import 'dev_replay.dart';

const agentEmptyLine =
    'Ask the agent to find, summarize, write or reorganize pages in this vault. Its edits show in page history like your own.';

/// The same transcript and composer serve the Agent section and editor dock.
class AgentChat extends StatefulWidget {
  const AgentChat({
    super.key,
    required this.store,
    this.panel = false,
    this.title,
  });
  final AgentStore store;
  final bool panel;

  /// The full-screen host's header. The dock supplies its own header.
  final String? title;
  @override
  State<AgentChat> createState() => _AgentChatState();
}

class _AgentChatState extends State<AgentChat> with TickerProviderStateMixin {
  /// Where the reader is, and how they move through the transcript. It holds
  /// the outline's folds and jumps.
  late final ChatReaderController _reader;

  /// The composer's text, so the approval bar knows when 1, 2 and 3 are
  /// not typing.
  final _draft = TextEditingController();

  /// The chat history setting at the last build, and where the reader was
  /// (message id and its top edge in the transcript viewport) when it last
  /// changed. The list built for the new mode opens there.
  bool? _wasSetting;
  ({String id, double y})? _switchAnchor;

  @override
  void initState() {
    super.initState();
    _reader = ChatReaderController(
      vsync: this,
      store: () => widget.store,
      presentation: () => _presentation,
      mounted: () => mounted,
      update: setState,
    );
    widget.store.showSession(true);
    widget.store.ensureOpen();
  }

  @override
  void didUpdateWidget(AgentChat oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.store != widget.store) {
      oldWidget.store.showSession(false);
      widget.store.showSession(true);
    }
  }

  @override
  void dispose() {
    widget.store.showSession(false);
    _reader.dispose();
    _draft.dispose();
    super.dispose();
  }

  /// Where this chat is shown. A reference names the presentation it is for.
  AgentPresentation get _presentation =>
      widget.panel ? AgentPresentation.dock : AgentPresentation.screen;

  /// What the outline's menus and open cards offer for a chapter, or for one
  /// of its sections.
  List<OutlineAction> _menuActions(
    OutlineChapter chapter,
    OutlineSection? section,
  ) => [
    OutlineAction(
      label: 'Add to note…',
      icon: Ico.plus,
      onSelected: () => _addToNote(chapter, section),
    ),
  ];

  /// Adds the chapter, or its section, to a note through the reference sheet.
  /// The kit's outline keeps its own types, so the raw chapter from the store
  /// is found by id.
  void _addToNote(OutlineChapter chapter, OutlineSection? section) {
    final store = widget.store;
    final sessionId = store.openId;
    final raw = store.outline.chapters
        .where((c) => c.id == chapter.id)
        .firstOrNull;
    if (sessionId == null || raw == null) return;
    final title = store.displaySessionTitle(
      store.openId ?? '',
      store.state?.session.title ?? '',
    );
    if (section == null) {
      showAddChatReference(
        context,
        ChatReference.fromChapter(
          sessionId: sessionId,
          chatTitle: title,
          chapter: raw,
        ),
      );
      return;
    }
    final rawSection = raw.sections
        .where((s) => s.id == section.id)
        .firstOrNull;
    if (rawSection == null) return;
    showAddChatReference(
      context,
      ChatReference.fromSection(
        sessionId: sessionId,
        chatTitle: title,
        chapter: raw,
        section: rawSection,
      ),
    );
  }

  /// The requests the agent waits on, above the composer: the first one
  /// answers to 1, 2 and 3.
  Widget _approvalBars(AgentStore store, double pad) {
    final pending = store.pendingApprovals;
    if (pending.isEmpty) return const SizedBox.shrink();
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: Layout.contentWidth + 64),
        child: Padding(
          padding: EdgeInsets.fromLTRB(pad, 0, pad, 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (var i = 0; i < pending.length; i++)
                Padding(
                  padding: EdgeInsets.only(top: i == 0 ? 0 : 6),
                  child: ApprovalBar(
                    key: ValueKey('agent-approval-${pending[i].id}'),
                    approval: kitApproval(pending[i]),
                    now: clockNow(),
                    captureKeys: i == 0,
                    composerEmpty: () => _draft.text.trim().isEmpty,
                    onDecide: (d) => store.decide(
                      pending[i].id,
                      switch (d.choice) {
                        ApprovalChoice.approve => 'approve',
                        ApprovalChoice.approveForSession => 'approve_session',
                        ApprovalChoice.deny => 'deny',
                      },
                      reason: d.reason,
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  /// The message at the top of the transcript and its top edge relative to the
  /// viewport, or null when none is measured.
  ({String id, double y})? _anchorInViewport() {
    final anchor = _reader.readingAnchor;
    final box = _reader.transcriptViewport.currentContext?.findRenderObject();
    if (anchor == null || box is! RenderBox || !box.hasSize) return null;
    return (id: anchor.id, y: anchor.y - box.localToGlobal(Offset.zero).dy);
  }

  /// Where the plain list opens after the setting changed: at the message the
  /// reader was on. Null otherwise, so the list follows the end as before.
  TranscriptAnchor? _plainAnchor(
    List<ChatItem> items,
    Map<String, String> anchors,
  ) {
    final switched = _switchAnchor;
    final item = switched == null ? null : anchors[switched.id];
    final at = item == null ? -1 : items.indexWhere((i) => i.id == item);
    // The row gap sits above the item inside its slot (none above the first).
    return switched == null || item == null || at < 0
        ? null
        : TranscriptAnchor(item, -(switched.y - (at == 0 ? 0 : 14)));
  }

  /// The compacted transcript: summary cards, the hairline and the live end.
  /// It opens on the hairline, or where the reader was before the setting
  /// changed.
  Widget _compactedList(
    AgentStore store,
    ChatTranscriptView rows,
    List<ChatItem> items,
    Map<String, String> anchors,
    EdgeInsets padding,
  ) {
    final history = store.compacted!;
    final built = compactedEntries(
      history: history,
      items: items,
      anchors: anchors,
      segmentOf: historySegments(history),
      controller: _reader.history,
      itemView: rows.itemView,
    );
    var initial = const TranscriptAnchor(historyBoundaryId);
    final switched = _switchAnchor;
    final entry = switched == null ? null : built.entryOf[switched.id];
    bool card(String? id) => id != null && id.startsWith('history-');
    double? gapBefore(String? previous, String id) =>
        card(previous) || card(id) ? 24 : null;
    if (switched != null && entry != null) {
      // A card opens at its top; a message where it was, the gap above it
      // being part of its slot.
      final at = built.entries.indexWhere((e) => e.id == entry);
      final gap = at <= 0
          ? 0.0
          : gapBefore(built.entries[at - 1].id, entry) ?? 14.0;
      initial = TranscriptAnchor(entry, card(entry) ? 0 : -(switched.y - gap));
    }
    return TranscriptList(
      key: const ValueKey('agent-history-list'),
      entries: built.entries,
      initialAnchor: initial,
      scroll: _reader.historyScroll,
      viewportKey: _reader.historyViewport,
      contentWidth: Layout.contentWidth,
      gutter: 0,
      rowGap: 14,
      gapBefore: gapBefore,
      padding: padding.copyWith(bottom: padding.bottom + 14),
    );
  }

  @override
  Widget build(BuildContext context) => ApprovalKeyPane(
    child: ListenableBuilder(
    listenable: widget.store,
    builder: (context, _) {
      final store = widget.store;
      final c = context.colors;
      if (_reader.stateFor != store.openId) {
        _reader.stateFor = store.openId;
        _reader.reset();
        _wasSetting = null;
        _switchAnchor = null;
      }
      final messages = store.state?.messages ?? const <AgentMessage>[];
      final outline = kitOutline(store.outline, messages: messages);
      final pendingReference = store.pendingFor(_presentation) != null;
      final compactedOn = showsCompacted(
        store,
        pendingReference: pendingReference,
      );
      // Only the reader changing the setting keeps their place; the cards
      // arriving with the data (or leaving for a reference) do not.
      final setting = store.historyCompacted(store.openId);
      if (_wasSetting != null && _wasSetting != setting) {
        _switchAnchor = _anchorInViewport();
      }
      _wasSetting = setting;
      _reader.compacted = compactedOn;
      final changed = !identical(_reader.rawOutline, store.outline);
      final sent =
          messages.length > _reader.lastMessages &&
          messages.lastOrNull?.role == 'user' &&
          !(store.replay?.active == true &&
              store.replay?.sessionId == store.openId);
      final keep =
          !compactedOn &&
              changed &&
              !sent &&
              !(_reader.onOutline ? _reader.atEnd : _reader.plainAtEnd) &&
              store.pendingFor(_presentation) == null
          ? _reader.readingAnchor
          : null;
      _reader.rawOutline = store.outline;
      final plainTop = !compactedOn && !_reader.indexed && !outline.isEmpty
          ? _reader.plainTop()
          : null;
      final fold = _reader.controller.fold;
      if (plainTop != null && !_reader.plainAtEnd) {
        // An already-closed chapter arriving around the reader stays open.
        final previous = _reader.controller.layout!;
        final item = previous.rows
            .whereType<ItemRow>()
            .where(
              (r) =>
                  'item:${previous.firstAnchorFor(r.itemId) ?? r.itemId.split('#').first}' ==
                  plainTop.key,
            )
            .firstOrNull;
        final msg =
            _reader.readingAnchor?.id ??
            (item == null ? null : previous.firstAnchorFor(item.itemId));
        final position = msg == null
            ? -1
            : messages.indexWhere((m) => m.id == msg);
        for (final chapter in outline.chapters) {
          final a = messages.indexWhere((m) => m.id == chapter.first);
          final b = messages.indexWhere((m) => m.id == chapter.last);
          if (!chapter.ongoing && a >= 0 && a <= position && b >= position) {
            fold.shown[chapter.first] = true;
            fold.deferred.add(chapter.first);
          }
        }
      }
      final messageIds = [for (final m in messages) m.id];
      final segments = outlineSegments(outline, messageIds);
      final anchors = <String, String>{};
      final items = agentTurns(
        store.state,
        streaming: store.streaming,
        error: store.error,
        anchorOf: anchors,
        segmentOf: compactedOn
            ? historySegments(store.compacted!)
            : segments.isEmpty
            ? null
            : segments,
      );
      final itemOf = {for (final item in items) item.id: item};
      // The anchor is measured after layout; walking children during build is
      // unsafe while the lazy transcript is replacing its rows.
      if (!compactedOn) {
        _reader.controller.sync(
          outline: outline,
          itemIds: [for (final item in items) item.id],
          anchorIds: messageIds,
          anchorOf: anchors,
          topAnchorId: _reader.readingAnchor?.id,
        );
      }
      final wasIndexed = _reader.indexed;
      _reader.indexed =
          _reader.indexed ||
          !outline.isEmpty ||
          store.pendingFor(_presentation) != null;
      if (_reader.indexed && !wasIndexed && plainTop != null) {
        final index = fold.indexOfKey(plainTop.key);
        if (index != null) {
          _reader.restore = (
            index: index,
            y: plainTop.y,
            globalY: plainTop.globalY,
          );
        }
      }
      _reader.follow(store);
      if (keep != null && _reader.indexed) {
        _reader.keepMessagePosition(
          keep,
          store.outline,
          afterInitialLayout: !wasIndexed,
        );
      }
      _reader.scheduleReaderMeasure();
      _reader.maybeTakePending();
      final rows = ChatTranscriptView(
        context: this.context,
        store: store,
        reader: _reader,
      );
      // Android stays in Hub mode; signed-out chat offers browser sign-in.
      final signIn = store.requiresHubSignIn;
      final notice = signIn
          ? null
          : !store.enabled
          ? 'The agent is off. Turn it on in Settings > AI.'
          : store.openError;
      final pad = widget.panel ? 12.0 : 32.0;
      final padding = EdgeInsets.fromLTRB(pad, widget.panel ? 14 : 28, pad, 16);
      final Widget body = signIn
          ? Center(
              child: Padding(
                padding: EdgeInsets.all(pad),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 460),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        'Sign in to your notes hub to chat with the agent. It reads and edits this vault there.',
                        key: const ValueKey('agent-sign-in-prompt'),
                        textAlign: TextAlign.center,
                        style: ts(context.type.body, c.textSecondary),
                      ),
                      const SizedBox(height: 16),
                      PrimaryButton(
                        key: const ValueKey('agent-sign-in'),
                        label: 'Sign in',
                        onTap: () =>
                            showSettings(context, section: SettingsSection.sync),
                      ),
                    ],
                  ),
                ),
              ),
            )
          : notice != null
          ? Center(
              child: Padding(
                padding: EdgeInsets.all(pad),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 460),
                  child: Text(
                    notice,
                    key: const ValueKey('agent-open-error'),
                    style: ts(context.type.body, c.textSecondary),
                  ),
                ),
              ),
            )
          : items.isEmpty
          ? Center(
              child: Padding(
                padding: EdgeInsets.all(pad),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 460),
                  child: Text(
                    agentEmptyLine,
                    key: const ValueKey('agent-empty'),
                    textAlign: TextAlign.center,
                    style: ts(context.type.body, c.textSecondary),
                  ),
                ),
              ),
            )
          : compactedOn
          ? _compactedList(store, rows, items, anchors, padding)
          : _reader.indexed
          ? OutlineList(
              controller: _reader.controller,
              padding: padding,
              initialIndex: _reader.restore?.index,
              initialY: _reader.restore?.y ?? 0,
              initialGlobalY: _reader.restore?.globalY,
              itemBuilder: (context, row) => switch (itemOf[row.itemId]) {
                final item? => rows.itemView(item),
                null => const SizedBox.shrink(),
              },
              chapterActions: (chapter) => _menuActions(chapter, null),
              sectionActions: _menuActions,
              cardActions: (chapter) => [
                OutlineAction(
                  label: 'Add to note',
                  onSelected: () => _addToNote(chapter, null),
                ),
              ],
            )
          : TranscriptList(
              entries: [
                for (var i = 0; i < items.length; i++)
                  TimelineEntry(
                    id: items[i].id,
                    builder: (_) => rows.plainRow(i, items[i]),
                  ),
              ],
              initialAnchor: _plainAnchor(items, anchors),
              scroll: _reader.scroll,
              viewportKey: _reader.plainViewport,
              contentWidth: Layout.contentWidth,
              gutter: 0,
              rowGap: 14,
              padding: padding.copyWith(bottom: padding.bottom + 14),
            );
      final switched = _switchAnchor;
      if (switched != null) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!mounted) return;
          _switchAnchor = null;
          // The outline list is indexed: put the message back where it was.
          final box = _reader.transcriptViewport.currentContext
              ?.findRenderObject();
          if (_reader.onOutline && box is RenderBox && box.hasSize) {
            _reader.keepMessagePosition((
              id: switched.id,
              y: box.localToGlobal(Offset.zero).dy + switched.y,
            ), store.outline, afterInitialLayout: true);
          }
        });
      }
      return Container(
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius: Radii.panel,
          boxShadow: widget.panel ? null : c.seam,
        ),
        child: ClipRRect(
          borderRadius: Radii.panel,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!widget.panel && widget.title != null)
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 12, 24, 10),
                  child: Text(
                    widget.title!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: tsDisplay(context.type.h3, c.textPrimary),
                  ),
                ),
              if (outline.isEmpty &&
                  !widget.panel &&
                  store.hostInfo?.kind != 'hub' &&
                  (store.hostInfo?.projects ?? false) &&
                  store.state?.session.projectId != null)
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 12, 24, 0),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: SelectButton<String>(
                      key: const ValueKey('agent-session-project'),
                      title: 'Project',
                      value: store.state!.session.projectId!,
                      options: [for (final p in store.projects) p.id],
                      labelOf: (id) =>
                          store.projects
                              .where((p) => p.id == id)
                              .firstOrNull
                              ?.name ??
                          store.state?.session.projectName ??
                          'Project',
                      onChanged: (id) =>
                          store.setSessionProject(store.state!.session.id, id),
                    ),
                  ),
                ),
              if (!signIn &&
                  notice == null &&
                  store.compactedFor == store.openId &&
                  hasHistoryCards(store.compacted))
                Padding(
                  padding: EdgeInsets.fromLTRB(pad, 4, pad, 0),
                  child: Align(
                    alignment: Alignment.centerRight,
                    child: ChatSettingsButton(store: store),
                  ),
                ),
              if (!outline.isEmpty && !compactedOn)
                OutlineWhereStrip(
                  where: _reader.controller.where,
                  onTap: () => _reader.controller.showContents(context),
                  colors: outlineStripColors(context, outline),
                ),
              Expanded(
                child: NotificationListener<ScrollNotification>(
                  onNotification: (_) {
                    _reader.scheduleReaderMeasure();
                    return false;
                  },
                  child: SizedBox.expand(
                    key: _reader.transcriptViewport,
                    child: body,
                  ),
                ),
              ),
              if (store.replay case final playback?
                  when playback.sessionId == store.openId)
                ReplayControls(playback: playback),
              if (!signIn) _approvalBars(store, pad),
              if (!signIn)
                Center(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(
                      maxWidth: Layout.contentWidth + 64,
                    ),
                    child: Padding(
                      padding: EdgeInsets.fromLTRB(
                        pad,
                        0,
                        pad,
                        widget.panel ? 12 : 20,
                      ),
                      child: AgentComposer(
                        store: store,
                        panel: widget.panel,
                        controller: _draft,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      );
    },
    ),
  );
}
