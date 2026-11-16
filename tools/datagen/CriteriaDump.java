// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the scoreboard objective criteria names vanilla 1.8.8 accepts
// (IScoreObjectiveCriteria.INSTANCES: dummy, trigger, health, kill and death counts, team
// kills and every statistic and achievement) from the SRG-remapped server jar (see
// AGENTS.md: reference/tools/server-srg.jar). Nothing from vanilla is copied: the
// registry is read through reflection after the server's own bootstrap and stat setup.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/CriteriaDump.java > criteria.txt
//
// Output: one name per line, sorted.

import java.util.Map;
import java.util.TreeSet;

public class CriteriaDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> criteria = Class.forName("net.minecraft.scoreboard.IScoreObjectiveCriteria");
        Map<?, ?> instances = (Map<?, ?>) criteria.getField("field_96643_a").get(null);
        TreeSet<String> names = new TreeSet<>();
        for (Object k : instances.keySet()) names.add((String) k);
        StringBuilder out = new StringBuilder();
        for (String n : names) out.append(n).append('\n');
        System.out.print(out);
    }
}
