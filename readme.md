# Winebasin

Winebasin uses overlays to allow you to use shared base Wine prefixes - saving space, hopefully allowing upgrades without full reinstall, and portable installations.

Winebasin uses these terms:

- **Basis** - the shared base prefix
- **System** - where you actually install software and run things, a layer on top of a basis

Here's how it works, roughly:

```shell
$ cargo install winebasin
# Create the basis, optionally install a billion (5+GB) of winetricks
$ winebasin basis create default --recommended-winetricks
# Create a system to install an app in
$ winebasin system create default my_app
# Start a shell with the correct env vars to install the app.
# Sudo is used for creating the overlay, everything else still runs as your user.
$ winebasin system shell my_app
[sudo] password for you: *******
> wine ~/Downloads/my_app.msi
> ^D
# Run the app using a prefix-relative path
$ winebasin system run my_app "Program Files/my_app/my_app.exe"
```

See `winebasin -h` for more details.

# How it works

The **basis** is a normal Wine prefix, set up like a normal Wine prefix.

When you run **system** commands, winebasin mounts an overlay filesystem combining the basis directory and system directory.

# What you are thinking right now

- Can I use this with Steam?

  No. This has a new workflow that's not a drop in replacement for similar Wine commands. It steams Steam may be working on something similar though.

- Mounting overlays requires sudo

  Yes. I heard there's a way to do this without sudo with namespaces but I'm not sure how exactly, and I feel like there's a catch that nobody mentions, like not having access to the rest of the filesystem or something.
