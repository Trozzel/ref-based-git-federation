# Following Remote Sites

This document describes the `.federation/following` file and how it affects
federation behavior.

## Purpose

`.federation/following` is a local policy file stored inside the repository
being managed by the federation agent.

It controls which remote sites this local site should follow for:

- adding new remotes when `SiteAdvertisement` messages arrive
- fetching updates when `RepoUpdated` messages arrive
- keeping or removing existing remotes when local follow preferences change

## File location

The file lives at:

```text
<repo_path>/.federation/following
```

If the `.federation` directory or `following` file does not exist, the agent
will treat that as "follow all sites".

When the agent creates the file automatically, the default contents are:

```text
*
```

## File format

The file is plain text.

Each non-empty line represents one site name to follow.

Example:

```text
site-a
site-b
site-c
```

## Rules

- Lines are trimmed before they are used.
- Blank lines are ignored.
- Duplicate site names are harmless.
- An existing but empty file means "follow no sites".
- A line containing `*` means "follow all sites".
- If the file is missing, behavior is the same as if the file contained `*`.

Examples:

```text
 alpha
beta
 gamma
```

This is treated as:

```text
alpha
beta
gamma
```

This:

```text
*
```

means all sites are followed.

This:

```text
```

when the file exists but contains no site names, means no sites are followed.

This:

```text
alpha
*
beta
```

also means all sites are followed.

## Runtime behavior

### SiteAdvertisement

When a `SiteAdvertisement` arrives over NATS:

1. The agent checks whether the advertised site is currently followed.
2. If it is not followed, the advertisement is ignored.
3. If it is followed, the agent checks whether that site is already configured
   as a git remote.
4. If the remote does not exist, it is added and fetched.

### RepoUpdated

When a `RepoUpdated` message arrives over NATS:

1. The agent checks whether the source site is currently followed.
2. If it is not followed, the update is ignored.
3. If it is followed, the existing remote is fetched.

### Follow file changes

The agent reacts to changes in follow policy in two ways:

- incoming `SiteAdvertisement` and `RepoUpdated` messages always use the latest
  follow rules
- a background reconciliation loop periodically rereads the file and removes
  remotes that are no longer followed

This means:

- removing a site from `.federation/following` will eventually remove its local
  remote
- adding a site to `.federation/following` allows future advertisements and
  updates from that site to be accepted
- leaving `.federation/following` empty will eventually remove all followed
  remotes except `origin`

## Important limitation

Adding a site name to `.federation/following` does not immediately create a new
remote by itself.

That is because the `following` file only contains a site name, not the remote
repository URL. A new remote is created when a matching `SiteAdvertisement`
arrives and supplies the `repo_url`.

## Farewell behavior

`SiteFarewell` messages are still honored independently of the following file.

If a remote site announces that it is leaving the federation, the corresponding
git remote is removed if it exists.

This is separate from local follow policy:

- `SiteFarewell` means the remote site is going away
- `.federation/following` means the local site does or does not want to follow
  that remote site
