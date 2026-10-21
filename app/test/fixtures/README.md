`chat_live_replay.sqlite` is synthetic replay data for `chat_live_replay_test.dart`.
It has eight recorded messages (three users, four assistants and one tool),
two closed subject chapters and three sections. Its tables match the readonly
BubblePlay replay schema consumed by the dev provider. The tool payload is
replaced by a harmless replay work step; recorded code is never executed.
