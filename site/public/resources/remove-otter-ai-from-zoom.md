# How to remove Otter AI from Zoom

Last reviewed: 2026-08-09

There is a ten-second fix that works even when you are not the host, and almost nobody knows it.

## The ten-second answer

Type `stop otter` into the Zoom meeting chat. The Notetaker leaves.

**Any participant can do this.** You do not need to be the host, and you do not need an Otter account. It works in Google Meet and Microsoft Teams chat too. This is the only removal method that requires no permissions at all, which makes it the one to remember when the bot belongs to somebody else in the call.

Caveat: if several Otter Notetakers are present because more than one attendee brought one, `stop otter` removes all of them. To remove a single Notetaker, use the participant-list method.

## If you are the host

1. Click **Participants** in the Zoom toolbar
2. Find the Notetaker in the list (it appears as a normal participant, named something like "Otter.ai Notetaker")
3. Click the **ellipsis** next to its name
4. Choose **Remove**

This ejects the bot from the current meeting only.

## Stop it coming back

In Otter: **Integrations → Meetings → Default auto-join settings → "Meetings I manually select."**

**The gotcha:** changing the default does *not* apply retroactively to calendar events you already customized. If you ever toggled auto-join on for a specific recurring meeting, that event keeps its own setting and Otter keeps joining it regardless of the new default. Open the calendar view in Otter and turn those events off individually. This is the most common reason people believe they disabled Otter and then watch it walk into a call the following week.

Stronger: disconnect the Google or Microsoft calendar from Otter entirely. A notetaker that cannot read your calendar cannot discover your meeting links, so there is nothing left to auto-join.

Otter's docs: https://help.otter.ai/hc/en-us/articles/12906714508823-Stop-Otter-Notetaker-from-automatically-joining-your-meetings and https://help.otter.ai/hc/en-us/articles/26010355877911-Choose-which-meetings-Otter-Notetaker-records

## Somebody else's Otter bot

You cannot reach into another person's Otter account and turn their bot off. In order of how well it works:

- **Ask.** "Could you drop the notetaker for this one?" is ordinary etiquette now, and the owner can remove it in one click. For a sensitive conversation this beats any technical control.
- **Type the chat command.** Works from any seat in the room.
- **Waiting Room plus authentication.** The strongest standing control. Notetaker bots cannot sign in to Zoom accounts, so requiring authenticated users blocks most of them outright.
- **Admin app controls.** Zoom admins can restrict or allow-list third-party apps under Admin → Advanced → App Marketplace, and audit installs under Apps on Account. The only option that scales past one meeting.

**The limit, stated plainly:** some notetakers no longer join as a separate participant. They capture audio through an attendee's own authenticated session, so there is no bot in the participant list and no Zoom setting that stops it. Waiting rooms and authentication defeat bots that dial in. Nothing defeats a participant who is recording. That has always been true of meetings; AI just made it cheaper.

## The version of this problem that solves itself

Every step above manages a symptom. The bot exists because cloud notetakers need your meeting audio on their servers, and joining as a synthetic participant is how they get it. Capture audio on your own device instead and the category disappears: nothing joins, nothing appears in the participant list, nothing needs admitting or ejecting.

That is how Minutes works: device-side recording, local transcription (whisper.cpp), markdown on your own disk. No bot and no cloud. Granola is also botless, though it transcribes in the cloud, which is a different trade.

One thing device-side capture does not change: tell people you are recording. The bot's single virtue was announcing itself; without it, consent is your job, which is where it belonged anyway.

## Related

- Other notetakers and platforms: https://useminutes.app/resources/remove-ai-notetaker-bots-from-meetings
- Otter vs Minutes: https://useminutes.app/compare/otter-vs-minutes
- Recording consent law by state: https://useminutes.app/resources/is-it-legal-to-record-a-meeting
- How botless capture works: https://useminutes.app/security
