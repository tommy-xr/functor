## Inspiration

The ideas behind Functor aren't new - it's a rehashing and recombination of several existing ideas that I'd like to share, and they all shine light on specific core principles of this project:
- __Immediate feedback:__ - 
  - Fast dev loop. changes to code should be reflected immediately - no long compile times, no taking off a headset to change a line of code and then put it back on, etc.
  - If it typechecks, it works
- __Understandability:__ - 
  - You should be able to understand the code you (or LLMs) write. 
  - That means being able to visualize data flow throughout the system.
- __LLM Assistance:__ - LLMs are extremely powerful. We should use them for the hard parts - debugging, writing boilerplate - and save our time and initiative for the most important and human aspects of our project.

Here are the pieces of work that have been personally inspiring to me that have helped form my plans for functor.

### Inspiration 1: Inventing on principle by Bret Victor

Link: [https://www.youtube.com/watch?v=PUv66718DII&t=2431s](Inventing on Principle)

This is just an incredible talk. If you haven't seen it, would recommend you go watch it right now, and come back.

The first time I saw this - it blew my mind, but the ideas also seemed so abstract (or at least, so far away from my day-to-day development experience).

### Inspiration 2: Tomorrow Corporation Tech Demo

Link: [https://www.youtube.com/watch?v=72y2EC5fkcE&pp=ygUedG9tb3Jyb3cgY29ycG9yYXRpb24gdGVjaCBkZW1v](Tomorrow Corporation Tech Demo)

This shows a demo of internal tooling of an indie game company - showcasing hot-reload, time travel, a powerful debugger. Showcases the power when you control the system end-to-end: runtime, debugger, etc.

### Inspiration 3: Light Table by Chris Granger

Link: [https://www.youtube.com/watch?v=H58-n7uldoU](Light Table)

This is an older project, and it's interesting because re-watching the video now - _many_ features that were innovative at the time have now found their ways into modern IDEs. However, one particularly powerful dimension - [visualizing code without a running debugger / live eval](http://youtube.com/watch?v=gtXpOD6jFls) - hasn't been mainstream, yet.

### Inspiration 4: Elm

[Elm](https://elm-lang.org/) was my first exposure to how you could take the "Inventing on Principle" ideas and _potentially make them a reality_ - seeing an actual [time travel debugger](https://elm-lang.org/news/time-travel-made-easy) that worked was incredible. This helped me understand how functional programming + immutability could be used to achieve some of my goals.

### Inspiration 5: React (and Redux)

[React](https://react.dev/) - before React, I didn't understand functional programming (coming from an OO background). How could pure functions do anything useful? However, when I saw the initial versions of React - where UI is a _pure function_ of state (and being burned by the complexity of manually managing the DOM) - it really clicked for me (personally though, I think newer React w/ hooks introduces a lot of accidental complexity - not a fan of any [feature that needs 'meta-rules' on how to use them](https://react.dev/reference/rules/rules-of-hooks)) 

[Redux](https://redux.js.org/) and the [great talk by Dan Abramov](https://www.youtube.com/watch?v=xsSnOQynTHs) was also influential in educating me about the power of functional programming concepts (and I eventually found out about Elm _from_ Redux). The challenge I had in using Redux _in practice_ was two-fold - the language was not ideal for it (lots of boilerplate in TS), and the managing of effects/coeffects was clunky (via middleware) - in practice, you could only use time travel for very limited examples.

Both React and Redux were my first taste into this sort of new paradigm.

## Some other thoughts

### Is Functor a good idea?

I'm actually not sure yet. It's an experiment. Maybe it's the wrong direction and API for building games. But I thought it'd be fun to try

### Are these ideas relevant in the LLM era?

In the back of mind, in the LLM era, I wonder: _are these ideas still relevant?_ It might be that they are not or will not be in the near future. However, I believe, at least for now, that they are: while LLMS can spit out prodigous amount of code and tackle increasingly hard problems, the output still needs to be tuned and refined. For a game: that means tuning the behavior and feel until the gameplay is exactly right.
