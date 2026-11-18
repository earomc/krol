// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A test-only Java agent for the vanilla 1.8.8 server used by the parity suite
// (AGENTS.md section 10.2). It adds one console command,
// `krolpin <seed> [randomTickSpeed] [ticks] [-- <command>]`, which seeds
// every world's random (World.rand), random block tick generator (World.updateLCG) and
// the generator behind Math.random(), then (after the command below) every mob's own
// random (Entity.rand of each EntityLiving, with the seed plus its entity id),
// and sets the cave mood countdown to 12000 (World.ambientTickCountdown). With a speed it
// also sets the randomTickSpeed game rule in the same tick, and with a tick count it sets
// the rule back to 0 at the end of that many ticks (through a tickable, which runs after
// the worlds' ticks), so a scenario can run random ticks for an exact number of ticks
// so that behavior driven by them can be compared with krol exactly. A command after
// `--` runs right after pinning, in the same tick (e.g. one that primes TNT).
// `krolpin mobs <seed> <ticks>` pins only the mobs' randoms, now and again at the end of
// each of the next ticks (through a tickable), so that what a bot does to a mob meets the
// same random state whichever tick it lands in. krol answers the
// same command in builds with the `parity` feature.
//
// The server jar is obfuscated, so the agent works through reflection on the official
// 1.8.8 names (looked up in the MCP mappings, see reference/tools):
//   net.minecraft.server.MinecraftServer.N()  getServer
//   MinecraftServer.P()                        getCommandManager (an ICommandManager `l`)
//   MinecraftServer.d                          worldServers (WorldServer[])
//   j.a(k)                                     CommandHandler.registerCommand(ICommand)
//   k                                          ICommand
//   adm.s / adm.m                              World.rand / World.updateLCG
//   adm.L                                      World.ambientTickCountdown
//   adm.Q() / adk.a(String, String)            World.getGameRules / GameRules.setOrCreateGameRule
//   MinecraftServer.a(km), km.c()              registerTickable(ITickable), ITickable.update
//   adm.f / pk / ps                            World.loadedEntityList / Entity / EntityLiving
//   pk.V / pk.F()                              Entity.rand / Entity.getEntityId
//
// Built by `cargo xtask parity` (javac --release 8) and loaded with -javaagent only for
// scenarios that set `pin_random`.

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.Collections;
import java.util.Random;

public final class KrolAgent {
    private static final String NAME = "krolpin";

    private KrolAgent() {}

    public static void premain(String args, Instrumentation inst) {
        Thread t = new Thread(KrolAgent::registerWhenReady, "krol-parity-agent");
        t.setDaemon(true);
        t.start();
    }

    /** Waits for the server object, then adds the command to its command manager. */
    private static void registerWhenReady() {
        try {
            Class<?> serverClass = Class.forName("net.minecraft.server.MinecraftServer");
            Method getServer = serverClass.getMethod("N");
            Method getCommands = serverClass.getMethod("P");
            Object server = null;
            Object commands = null;
            while (commands == null) {
                server = getServer.invoke(null);
                if (server != null) {
                    commands = getCommands.invoke(server);
                }
                if (commands == null) {
                    Thread.sleep(5);
                }
            }
            Class<?> command = Class.forName("k");
            Class<?> handler = Class.forName("j");
            final Object theServer = server;
            final Object theCommands = commands;
            InvocationHandler behavior =
                (proxy, method, a) -> answer(theServer, theCommands, proxy, method, a);
            Object pin = Proxy.newProxyInstance(command.getClassLoader(), new Class<?>[] {command}, behavior);
            handler.getMethod("a", command).invoke(commands, pin);
        } catch (Exception e) {
            System.err.println("krol parity agent failed: " + e);
        }
    }

    /** The ICommand methods of the pin command, told apart by name and parameters. */
    private static Object answer(Object server, Object commands, Object self, Method m, Object[] a)
            throws Exception {
        String n = m.getName();
        int params = m.getParameterCount();
        Class<?> ret = m.getReturnType();
        if (n.equals("c") && params == 0) {
            return NAME; // getCommandName
        }
        if (n.equals("c") && params == 1) {
            return "/" + NAME + " <seed>"; // getCommandUsage
        }
        if (n.equals("b") && params == 0) {
            return Collections.emptyList(); // getCommandAliases
        }
        if (n.equals("a") && params == 1 && ret == boolean.class) {
            return true; // canCommandSenderUseCommand
        }
        if (n.equals("a") && params == 2 && ((String[]) a[1]).length >= 3 && ((String[]) a[1])[0].equals("mobs")) {
            // krolpin mobs <seed> <ticks>: pin every mob's random now and at the end of
            // each of the next ticks.
            String[] args = (String[]) a[1];
            long seed = Long.parseLong(args[1]);
            pinMobs(server, seed);
            repinMobs(server, seed, Integer.parseInt(args[2]));
            return null;
        }
        if (n.equals("a") && params == 2) {
            String[] all = (String[]) a[1]; // processCommand
            // Everything after "--" is a command to run right after pinning.
            int dash = java.util.Arrays.asList(all).indexOf("--");
            String[] args = dash < 0 ? all : java.util.Arrays.copyOf(all, dash);
            pin(server, Long.parseLong(args[0]));
            if (args.length >= 2) {
                setRandomTickSpeed(server, args[1]);
            }
            if (args.length >= 3) {
                stopAfter(server, Integer.parseInt(args[2]));
            }
            if (dash >= 0) {
                String then = String.join(" ", java.util.Arrays.copyOfRange(all, dash + 1, all.length));
                Class.forName("j").getMethod("a", Class.forName("m"), String.class).invoke(commands, a[0], then);
            }
            pinMobs(server, Long.parseLong(args[0]));
            return null;
        }
        if (n.equals("a") && params == 3) {
            return null; // addTabCompletionOptions
        }
        if (n.equals("b") && params == 2) {
            return false; // isUsernameIndex
        }
        if (n.equals("compareTo")) {
            Object name = a[0].getClass().getMethod("c").invoke(a[0]);
            return NAME.compareTo((String) name);
        }
        if (n.equals("hashCode")) {
            return System.identityHashCode(self);
        }
        if (n.equals("equals")) {
            return self == a[0];
        }
        if (n.equals("toString")) {
            return NAME;
        }
        throw new UnsupportedOperationException(n);
    }

    /** Sets the randomTickSpeed game rule (shared by all worlds). */
    private static void setRandomTickSpeed(Object server, String speed) throws Exception {
        Object overworld = worlds(server)[0];
        Object rules = Class.forName("adm").getMethod("Q").invoke(overworld);
        rules.getClass().getMethod("a", String.class, String.class).invoke(rules, "randomTickSpeed", speed);
    }

    /** Sets randomTickSpeed to 0 at the end of the `ticks`-th tick from now. */
    private static void stopAfter(Object server, int ticks) throws Exception {
        Class<?> tickable = Class.forName("km");
        final int[] left = {ticks};
        InvocationHandler count = (proxy, method, a) -> {
            if (method.getName().equals("c") && left[0] > 0 && --left[0] == 0) {
                setRandomTickSpeed(server, "0");
            }
            if (method.getName().equals("hashCode")) {
                return System.identityHashCode(proxy);
            }
            if (method.getName().equals("equals")) {
                return proxy == a[0];
            }
            return null;
        };
        Object hook = Proxy.newProxyInstance(tickable.getClassLoader(), new Class<?>[] {tickable}, count);
        Class.forName("net.minecraft.server.MinecraftServer").getMethod("a", tickable).invoke(server, hook);
    }

    /**
     * Seeds the own random (Entity.rand) of every mob (EntityLiving) in every world with
     * the seed plus its entity id, after the command that follows "--" (which may
     * summon it).
     */
    private static void pinMobs(Object server, long seed) throws Exception {
        Class<?> world = Class.forName("adm");
        Field loaded = world.getDeclaredField("f");
        loaded.setAccessible(true);
        Class<?> entity = Class.forName("pk");
        Class<?> mob = Class.forName("ps");
        Field rand = entity.getDeclaredField("V");
        rand.setAccessible(true);
        Method entityId = entity.getMethod("F");
        for (Object w : worlds(server)) {
            if (w == null) {
                continue;
            }
            for (Object e : (java.util.List<?>) loaded.get(w)) {
                if (mob.isInstance(e)) {
                    ((Random) rand.get(e)).setSeed(seed + (int) entityId.invoke(e));
                }
            }
        }
    }

    /** Pins every mob's random at the end of each of the next `ticks` ticks. */
    private static void repinMobs(Object server, long seed, int ticks) throws Exception {
        Class<?> tickable = Class.forName("km");
        final int[] left = {ticks};
        InvocationHandler repin = (proxy, method, a) -> {
            if (method.getName().equals("c") && left[0] > 0) {
                left[0]--;
                pinMobs(server, seed);
            }
            if (method.getName().equals("hashCode")) {
                return System.identityHashCode(proxy);
            }
            if (method.getName().equals("equals")) {
                return proxy == a[0];
            }
            return null;
        };
        Object hook = Proxy.newProxyInstance(tickable.getClassLoader(), new Class<?>[] {tickable}, repin);
        Class.forName("net.minecraft.server.MinecraftServer").getMethod("a", tickable).invoke(server, hook);
    }

    private static Object[] worlds(Object server) throws Exception {
        Field worldsField = server.getClass().getSuperclass().getDeclaredField("d");
        worldsField.setAccessible(true);
        return (Object[]) worldsField.get(server);
    }

    /** Seeds World.rand and World.updateLCG of every loaded world, and Math.random(). */
    private static void pin(Object server, long seed) throws Exception {
        Object[] worlds = worlds(server);
        Class<?> world = Class.forName("adm");
        Field rand = world.getDeclaredField("s");
        rand.setAccessible(true);
        Field lcg = world.getDeclaredField("m");
        lcg.setAccessible(true);
        Field mood = world.getDeclaredField("L");
        mood.setAccessible(true);
        for (Object w : worlds) {
            if (w == null) {
                continue;
            }
            ((Random) rand.get(w)).setSeed(seed);
            lcg.setInt(w, (int) seed);
            mood.setInt(w, 12000);
        }
        // Math.random() draws from one shared generator (primed TNT picks its motion
        // with it).
        Field mathRandom = Class.forName("java.lang.Math$RandomNumberGeneratorHolder")
            .getDeclaredField("randomNumberGenerator");
        mathRandom.setAccessible(true);
        ((Random) mathRandom.get(null)).setSeed(seed);
    }
}
