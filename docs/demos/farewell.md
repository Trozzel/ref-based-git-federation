

# farewell demo script

## tab 1

```
make cycle
```

## sites - tabs  2 - 4

```
make site<id>
```

## tab 5 (or tab 1?)

show the current remotes / can also show the log messages via the various site specific tabs

```
make remotes
```

## switch to tab 3 (site 2)

ctrl-c to kill the agent

```
# send farewell message for site 2

make farewell2
```

observe the remotes for sites 1 and 3 are dropped from site 2

switch tabs to site 1 | site 3, observe the logs show site 2 was removed

from any available tab run

```
make remotes
```

should show each site and what remotes are still registered:
    - site 1 should only have site 3
    - site 2 shouldn't have any
    - site 3 should only have site 1


## adding site 2 back

if you want to show site 2 re-joining after a farewell you can just run the following command again:

```
make site2
```

and then look at other sites / remotes

```
make remotes
```

