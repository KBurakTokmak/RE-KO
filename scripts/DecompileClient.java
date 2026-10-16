// @category RE-KO
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import java.nio.file.Files;
import java.nio.file.Path;

public class DecompileClient extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        Path output = Path.of(args[0]);
        Files.createDirectories(output);
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        try {
            for (int i = 1; i < args.length; i++) {
                Address address = toAddr(Long.decode(args[i]));
                disassemble(address);
                Function function = getFunctionAt(address);
                if (function == null) function = getFunctionContaining(address);
                if (function == null) function = createFunction(address, "client_" + address);
                if (function == null) throw new Exception("No function at " + address);
                DecompileResults result = decompiler.decompileFunction(function, 60, monitor);
                if (!result.decompileCompleted()) throw new Exception(result.getErrorMessage());
                Files.writeString(output.resolve(address + ".c"), result.getDecompiledFunction().getC());
                println("Decompiled " + address);
            }
        } finally {
            decompiler.dispose();
        }
    }
}
