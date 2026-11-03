We need to ensure that when a new client connects, the server correctly sends the `pause` action with the actual current state of the player, and not auto-play immediately when the user is paused.
Currently, `client.py` has `self._pending_pause = False` hardcoded when receiving an `action == 'play'`, so if the current server state is paused, the play action starts the video, but the `_pending_pause = False` will cause it to unpause as soon as it loads.
We need to change `self._pending_pause = data.get('pause_state', False)` in the `play` action. Let's see if we can do this.
