import '../../services/addon_services.dart';

import 'package:flutter/widgets.dart';

import '../../ui/anchored_list.dart';
import '../../ui/ui.dart';
import 'add_to_note.dart';
import 'agent_composer.dart';
import 'agent_outline.dart';
import 'agent_outline_view.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'chat_reader_controller.dart';
import 'chat_reference.dart';
import 'chat_transcript_view.dart';
import 'dev_replay.dart';
import 'outline_read_state.dart';

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
  /// What the reader has folded, opened and collapsed in the outline.
  late final OutlineReadState _read;

  /// Where the reader is, and how they move through the transcript.
  late final ChatReaderController _reader;

  @override
  void initState() {
    super.initState();
    _read = OutlineReadState(
      vsync: this,
      mounted: () => mounted,
      update: setState,
      pinTop: () => _reader.list.pinTop(),
    );
    _reader = ChatReaderController(
      read: _read,
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
    _read.dispose();
    _reader.dispose();
    super.dispose();
  }

  /// Where this chat is shown. A reference names the presentation it is for.
  AgentPresentation get _presentation =>
      widget.panel ? AgentPresentation.dock : AgentPresentation.screen;

  /// Opens the contents sheet; a tap there reveals the chapter or section.
  void _openContents() {
    showOutlineContents(
      context,
      outline: _reader.outline,
      where: _reader.where.value,
      onChapter: _reader.openChapter,
      onSection: (chapter, section) => _reader.jump(
        section.first,
        section.last,
        sectionKey: 'sec:${section.id}',
      ),
    );
  }

  void _chapterMenu(ChapterView chapter) {
    showOutlineMenu(
      context,
      actions: [('Add to note…', Ico.plus, () => _addToNote(chapter, null))],
    );
  }

  void _sectionMenu(ChapterView chapter, SectionView section) {
    showOutlineMenu(
      context,
      actions: [('Add to note…', Ico.plus, () => _addToNote(chapter, section))],
    );
  }

  /// Adds the chapter, or its section, to a note through the reference sheet.
  /// The outline keeps its own view types, so the raw chapter from the store
  /// is found by id.
  void _addToNote(ChapterView chapter, SectionView? section) {
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

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.store,
    builder: (context, _) {
      final store = widget.store;
      final c = context.colors;
      if (_reader.stateFor != store.openId) {
        _reader.stateFor = store.openId;
        _read.reset();
        _reader.reset();
      }
      final messages = store.state?.messages ?? const <AgentMessage>[];
      final outline = store.outline.chapters.isEmpty
          ? OutlineView.empty
          : OutlineView.of(store.outline, messages: messages);
      final changed = !identical(_reader.rawOutline, store.outline);
      final sent =
          messages.length > _reader.lastMessages &&
          messages.lastOrNull?.role == 'user' &&
          !(store.replay?.active == true &&
              store.replay?.sessionId == store.openId);
      final keep =
          changed &&
              !sent &&
              !(_reader.indexed ? _reader.atEnd : _reader.plainAtEnd) &&
              store.pendingFor(_presentation) == null
          ? _reader.readingAnchor
          : null;
      _reader.rawOutline = store.outline;
      final plainTop = !_reader.indexed && !outline.isEmpty
          ? _reader.plainTop()
          : null;
      if (plainTop != null && !_reader.plainAtEnd) {
        // An already-closed chapter arriving around the reader stays open.
        final item = _reader.layout!.rows
            .whereType<ItemRow>()
            .where(
              (r) =>
                  'item:${_reader.layout!.firstMessageFor(r.item.id) ?? r.item.id.split('#').first}' ==
                  plainTop.key,
            )
            .firstOrNull;
        final msg =
            _reader.readingAnchor?.id ??
            (item == null
                ? null
                : _reader.layout!.firstMessageFor(item.item.id));
        final position = msg == null
            ? -1
            : messages.indexWhere((m) => m.id == msg);
        for (final chapter in outline.chapters) {
          final a = messages.indexWhere((m) => m.id == chapter.first);
          final b = messages.indexWhere((m) => m.id == chapter.last);
          if (!chapter.ongoing && a >= 0 && a <= position && b >= position) {
            _read.shown[chapter.first] = true;
            _read.deferred.add(chapter.first);
          }
        }
      }
      // The anchor is measured after layout; walking children during build is
      // unsafe while the lazy transcript is replacing its rows.
      _read.closeFolds(
        outline,
        _reader.chaptersInView(),
        messages,
        _reader.readingAnchor?.id,
      );
      _reader.outline = outline;
      final messageIds = [for (final m in messages) m.id];
      final segments = outlineSegments(outline, messageIds);
      final anchors = <String, String>{};
      final items = agentTurns(
        store.state,
        streaming: store.streaming,
        error: store.error,
        approvals: store.pendingApprovals,
        anchorOf: anchors,
        segmentOf: segments.isEmpty ? null : segments,
      );
      final layout = OutlineLayout.build(
        outline: outline,
        items: items,
        messageIds: messageIds,
        anchorOf: anchors,
        folded: _read.hidden,
      );
      _reader.layout = layout;
      _read.rekey(layout, outline);
      final wasIndexed = _reader.indexed;
      _reader.indexed =
          _reader.indexed ||
          !outline.isEmpty ||
          store.pendingFor(_presentation) != null;
      if (_reader.indexed && !wasIndexed && plainTop != null) {
        final index = _read.indexOfKey(plainTop.key);
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
      if (!outline.isEmpty) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _reader.refreshWhere();
        });
      }
      _reader.maybeTakePending();
      final rows = ChatTranscriptView(
        context: this.context,
        store: store,
        reader: _reader,
        read: _read,
        onChapterMenu: _chapterMenu,
        onSectionMenu: _sectionMenu,
        onAddToNote: _addToNote,
        onExpandedChanged: (first, open) =>
            setState(() => _read.expanded[first] = open),
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
                            showSettings(context, section: SettingsSection.ai),
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
          : _reader.indexed
          ? AnchoredList(
              controller: _reader.list,
              itemCount: layout.rows.length,
              padding: padding,
              onVisibleRangeChanged: _reader.onVisible,
              keyOf: (i) =>
                  i < _read.stableKeys.length ? _read.stableKeys[i] : null,
              indexOf: _read.indexOfKey,
              initialIndex: _reader.restore?.index,
              initialY: _reader.restore?.y ?? 0,
              initialGlobalY: _reader.restore?.globalY,
              itemBuilder: (context, i) => rows.rowView(layout.rows[i]),
            )
          : ListView.builder(
              key: _reader.plainViewport,
              controller: _reader.scroll,
              padding: padding,
              itemCount: items.length,
              itemBuilder: (context, i) => rows.plainRow(i, items[i]),
            );
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
              if (!outline.isEmpty)
                OutlineWhereStrip(
                  where: _reader.where,
                  onTap: _openContents,
                  colors: {
                    for (var i = 0; i < outline.chapters.length; i++)
                      outline.chapters[i].id: outlineColor(context, i),
                  },
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
                      child: AgentComposer(store: store, panel: widget.panel),
                    ),
                  ),
                ),
            ],
          ),
        ),
      );
    },
  );
}
