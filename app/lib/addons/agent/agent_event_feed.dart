import 'dart:async';

import '../../src/rust/api/agent.dart';

/// The store's event feed for one connection. Replacing a Rust subscription
/// (local viewing changes call connection-wide closeEvents) keeps readers
/// alive, but tells them to reload across the replacement window.
class AgentEventFeed {
  Object? _connection;
  StreamController<AgentEvent>? _events;
  int _subscription = 0;
  bool _bound = false;

  /// Publish the connection before opening work can notify the pane.
  void open(Object connection) {
    if (identical(connection, _connection)) return;
    final old = _events;
    if (old != null) unawaited(old.close());
    _connection = connection;
    _events = StreamController<AgentEvent>.broadcast();
    _bound = false;
    _subscription++;
  }

  Stream<AgentEvent> eventsFor(Object connection) =>
      identical(connection, _connection)
      ? _events!.stream
      : const Stream<AgentEvent>.empty();

  StreamSubscription<AgentEvent> bind(
    Object connection,
    Stream<AgentEvent> events, {
    required void Function(AgentEvent) onEvent,
    required void Function(Object) onError,
  }) {
    open(connection);
    final replacing = _bound;
    _bound = true;
    final controller = _events!;
    final subscription = ++_subscription;
    bool current() =>
        identical(controller, _events) && subscription == _subscription;
    final result = events.listen(
      (event) {
        if (!current()) return;
        controller.add(event);
        onEvent(event);
      },
      onError: (Object error, StackTrace stack) {
        if (!current()) return;
        controller.addError(error, stack);
        onError(error);
      },
    );
    if (replacing) {
      controller.add(
        const AgentEvent(
          kind: 'gap',
          sequence: 0,
          durable: false,
          sessionId: '',
          runId: '',
          createdAt: '',
          isError: false,
          skipped: 0,
          json: '{}',
        ),
      );
    }
    return result;
  }

  void close(Object connection) {
    if (!identical(connection, _connection)) return;
    _connection = null;
    _bound = false;
    final events = _events;
    _events = null;
    _subscription++;
    if (events != null) unawaited(events.close());
  }
}
