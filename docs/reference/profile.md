# @neyuki/profile

A simple profiler that counts calls and measures the time spent in each function, to find what makes a script slow.

```lua
local profile = require("@neyuki/profile")

profile.start()
runSimulation()
print(profile.stop())
-- runSimulation: calls=1, time=0.84s
-- step: calls=1000, time=0.81s
-- ...
```

## profile.start(): ()

Starts profiling, discarding earlier results.

## profile.stop(): string

Stops profiling and returns one `name: calls=N, time=Ts` line per function, slowest first. Times include the functions each one called.

## profile.isRunning(): boolean

Whether profiling is on.
