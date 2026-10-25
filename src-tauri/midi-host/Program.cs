using System.Text.Json;
using System.Runtime.InteropServices;
using Windows.Foundation;
using Windows.Devices.Midi2;
using Windows.Devices.Midi2.Enumeration;
using Windows.Devices.Midi2.Transports.Virtual;

if (args.Contains("--verify-bundle", StringComparer.Ordinal)) return;

var backendName = args.Contains("--backend=windowsMidiServices", StringComparer.Ordinal)
    ? "windowsMidiServices"
    : "teVirtualMidi";
var jsonOptions = new JsonSerializerOptions(JsonSerializerDefaults.Web);
IHostedMidiBackend backend = backendName == "windowsMidiServices"
    ? new WindowsMidiServicesBackend()
    : new TeVirtualMidiBackend();
var running = true;

while (running && Console.ReadLine() is { } line)
{
    string id = "";
    string? command = null;
    try
    {
        using var document = JsonDocument.Parse(line);
        var root = document.RootElement;
        id = root.TryGetProperty("id", out var idValue) ? idValue.GetString() ?? "" : "";
        command = root.TryGetProperty("command", out var commandValue) ? commandValue.GetString() : null;
        object response = command switch
        {
            "status" => new { ok = true, available = backend.Available, message = backend.Message, backend = backendName },
            "list" => new { ok = true, ports = backend.Ports.Select(p => new { id = p.Id, name = p.Name }) },
            "create" => new { ok = true, port = backend.Create(root.GetProperty("name").GetString() ?? "") },
            "delete" => backend.Delete(root.GetProperty("portId").GetString() ?? "")
                ? new { ok = true, message = "Port removed." }
                : new { ok = false, error = "The port no longer exists." },
            "shutdown" => Shutdown(),
            _ => new { ok = false, error = "Unknown MIDI host command." }
        };
        Console.WriteLine(JsonSerializer.Serialize(new { id, response }, jsonOptions));
        if (command == "shutdown") running = false;
    }
    catch (Exception ex)
    {
        Console.WriteLine(JsonSerializer.Serialize(new { id, response = new { ok = false, error = ex.Message } }, jsonOptions));
    }
}

backend.Dispose();

object Shutdown()
{
    backend.Dispose();
    return new { ok = true };
}

interface IHostedMidiBackend : IDisposable
{
    bool Available { get; }
    string Message { get; }
    IReadOnlyCollection<HostedMidiPort> Ports { get; }
    HostedMidiPort Create(string name);
    bool Delete(string id);
}

sealed record HostedMidiPort(string Id, string Name);

sealed class WindowsMidiServicesBackend : IHostedMidiBackend
{
    private readonly Dictionary<string, WindowsMidiPort> _ports = new(StringComparer.Ordinal);
    private readonly MidiSession? _session;
    public bool Available => _initializationError is null && _session is not null;
    public string Message => _initializationError ?? "Windows MIDI Services is ready.";
    private readonly string? _initializationError;
    private bool _disposed;
    public IReadOnlyCollection<HostedMidiPort> Ports => _ports.Values.Select(p => p.Info).ToArray();

    public WindowsMidiServicesBackend()
    {
        try
        {
            if (!MidiApi.EnsureServiceAvailable()) _initializationError = "Windows MIDI Services could not be started.";
            else if (!MidiVirtualDeviceManager.IsTransportAvailable) _initializationError = "The Windows MIDI virtual-device transport is unavailable.";
            else _session = MidiSession.Create("midi-fiesta");
        }
        catch (Exception ex) { _initializationError = ex.Message; }
        if (_session is null && _initializationError is null) _initializationError = "Windows MIDI Services did not create a session.";
    }

    public HostedMidiPort Create(string name)
    {
        if (!Available) throw new InvalidOperationException(Message);
        ValidateName(name, _ports.Values.Select(p => p.Info));

        var id = Guid.NewGuid().ToString("N");
        var endpointInfo = new MidiDeclaredEndpointInfo
        {
            Name = name,
            ProductInstanceId = $"MIDI-FIESTA-{id}",
            SpecificationVersionMajor = 1,
            SpecificationVersionMinor = 1,
            SupportsMidi10Protocol = true,
            SupportsMidi20Protocol = false,
            HasStaticFunctionBlocks = true,
            DeclaredFunctionBlockCount = 1
        };
        var config = new MidiVirtualDeviceCreationConfig(name, "Temporary virtual MIDI port created by midi-fiesta", "midi-fiesta", endpointInfo)
        {
            CreateOnlyUmpEndpoints = false
        };
        config.FunctionBlocks.Add(new MidiFunctionBlock
        {
            Number = 0,
            Name = name,
            IsActive = true,
            UIHint = MidiFunctionBlockUIHint.Sender,
            FirstGroup = new MidiGroup(0),
            GroupCount = 1,
            Direction = MidiFunctionBlockDirection.Bidirectional,
            RepresentsMidi10Connection = MidiFunctionBlockRepresentsMidi10Connection.YesBandwidthUnrestricted,
            MaxSystemExclusive8Streams = 0,
            MidiCIMessageVersionFormat = 0
        });

        var device = MidiVirtualDeviceManager.CreateVirtualDevice(config)
            ?? throw new InvalidOperationException("Windows MIDI Services could not create the virtual port.");
        MidiEndpointConnection? connection = null;
        try
        {
            connection = _session!.CreateEndpointConnection(device.DeviceEndpointDeviceId)
                ?? throw new InvalidOperationException("Could not connect to the virtual MIDI port.");
            if (connection.AddMessageProcessingPlugin(device) != MidiMessageProcessingPluginAddResult.Succeeded)
                throw new InvalidOperationException("Could not initialize the virtual MIDI port.");
            void RelayMidi(IMidiMessageReceivedEventSource _, MidiMessageReceivedEventArgs args)
            {
                var packet = args.GetMessagePacket();
                connection.SendSingleMessagePacket((IMidiUniversalPacket)packet);
            }
            connection.MessageReceived += RelayMidi;
            if (!connection.Open())
            {
                connection.MessageReceived -= RelayMidi;
                throw new InvalidOperationException("Could not open the virtual MIDI port.");
            }
            var hosted = new WindowsMidiPort(new HostedMidiPort(id, name), _session, connection, device, RelayMidi);
            _ports.Add(id, hosted);
            return hosted.Info;
        }
        catch
        {
            if (connection is not null) _session!.DisconnectEndpointConnection(connection.ConnectionId);
            throw;
        }
    }

    public bool Delete(string id)
    {
        if (!_ports.Remove(id, out var port)) return false;
        port.Dispose();
        return true;
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        foreach (var port in _ports.Values) port.Dispose();
        _ports.Clear();
        _session?.Dispose();
    }

    private sealed class WindowsMidiPort(HostedMidiPort info, MidiSession session, MidiEndpointConnection connection,
        MidiVirtualDevice device, Windows.Foundation.TypedEventHandler<IMidiMessageReceivedEventSource, MidiMessageReceivedEventArgs> handler) : IDisposable
    {
        public HostedMidiPort Info { get; } = info;
        private readonly MidiVirtualDevice _device = device;
        public void Dispose()
        {
            connection.MessageReceived -= handler;
            session.DisconnectEndpointConnection(connection.ConnectionId);
        }
    }

    internal static void ValidateName(string name, IEnumerable<HostedMidiPort> ports)
    {
        if (string.IsNullOrWhiteSpace(name)) throw new InvalidOperationException("Enter a port name.");
        if (name.Length > 31) throw new InvalidOperationException("Port names must be 31 characters or fewer.");
        if (ports.Any(p => string.Equals(p.Name, name, StringComparison.OrdinalIgnoreCase)))
            throw new InvalidOperationException("A port with that name already exists.");
    }
}

sealed class TeVirtualMidiBackend : IHostedMidiBackend
{
    private readonly Dictionary<string, TeVirtualMidiPort> _ports = new(StringComparer.Ordinal);
    private readonly string? _initializationError;
    public bool Available => _initializationError is null;
    public string Message => _initializationError ?? "teVirtualMIDI interface loaded. The driver is checked when a port is created.";
    public IReadOnlyCollection<HostedMidiPort> Ports => _ports.Values.Select(p => p.Info).ToArray();

    public TeVirtualMidiBackend()
    {
        try { _ = TeVirtualMidiApi.GetVersion(); }
        catch (Exception ex) { _initializationError = ex.Message; }
    }

    public HostedMidiPort Create(string name)
    {
        if (!Available) throw new InvalidOperationException(Message);
        WindowsMidiServicesBackend.ValidateName(name, _ports.Values.Select(p => p.Info));
        var port = new TeVirtualMidiPort(name);
        _ports.Add(port.Info.Id, port);
        return port.Info;
    }

    public bool Delete(string id)
    {
        if (!_ports.Remove(id, out var port)) return false;
        port.Dispose();
        return true;
    }

    public void Dispose()
    {
        foreach (var port in _ports.Values) port.Dispose();
        _ports.Clear();
    }
}

sealed class TeVirtualMidiPort : IDisposable
{
    private const uint MaxSysexLength = 65535;
    private const uint ParseIncomingCommands = 1;
    private IntPtr _handle;
    private readonly Thread _reader;
    private volatile bool _stopping;
    public HostedMidiPort Info { get; }

    public TeVirtualMidiPort(string name)
    {
        _handle = TeVirtualMidiApi.CreatePort(name, IntPtr.Zero, IntPtr.Zero, MaxSysexLength, ParseIncomingCommands);
        if (_handle == IntPtr.Zero)
        {
            var error = Marshal.GetLastPInvokeError();
            throw new InvalidOperationException(TeVirtualMidiApi.DescribeError(error));
        }
        Info = new HostedMidiPort(Guid.NewGuid().ToString("N"), name);
        _reader = new Thread(ReadAndRelayMidi) { IsBackground = true, Name = $"midi-fiesta:{name}" };
        _reader.Start();
    }

    private void ReadAndRelayMidi()
    {
        var buffer = new byte[checked((int)MaxSysexLength)];
        while (!_stopping)
        {
            uint length = (uint)buffer.Length;
            if (!TeVirtualMidiApi.GetData(_handle, buffer, ref length))
            {
                if (!_stopping) Console.Error.WriteLine($"teVirtualMIDI receive failed: {TeVirtualMidiApi.DescribeError(Marshal.GetLastPInvokeError())}");
                return;
            }
            if (length == 0) continue;
            if (!TeVirtualMidiApi.SendData(_handle, buffer, length))
            {
                if (!_stopping) Console.Error.WriteLine($"teVirtualMIDI relay failed: {TeVirtualMidiApi.DescribeError(Marshal.GetLastPInvokeError())}");
                return;
            }
        }
    }

    public void Dispose()
    {
        if (_handle == IntPtr.Zero) return;
        _stopping = true;
        _ = TeVirtualMidiApi.Shutdown(_handle);
        _reader.Join();
        TeVirtualMidiApi.ClosePort(_handle);
        _handle = IntPtr.Zero;
    }
}

static class TeVirtualMidiApi
{
    private const string DllName = "teVirtualMIDI.dll";
    [System.Runtime.InteropServices.DllImport(DllName, EntryPoint = "virtualMIDICreatePortEx2", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
    internal static extern IntPtr CreatePort(string name, IntPtr callback, IntPtr callbackContext, uint maxSysexLength, uint flags);
    [System.Runtime.InteropServices.DllImport(DllName, EntryPoint = "virtualMIDIGetData", SetLastError = true)]
    [return: System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.Bool)]
    internal static extern bool GetData(IntPtr port, [System.Runtime.InteropServices.Out] byte[] data, ref uint length);
    [System.Runtime.InteropServices.DllImport(DllName, EntryPoint = "virtualMIDISendData", SetLastError = true)]
    [return: System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.Bool)]
    internal static extern bool SendData(IntPtr port, byte[] data, uint length);
    [System.Runtime.InteropServices.DllImport(DllName, EntryPoint = "virtualMIDIShutdown", SetLastError = true)]
    [return: System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.Bool)]
    internal static extern bool Shutdown(IntPtr port);
    [System.Runtime.InteropServices.DllImport(DllName, EntryPoint = "virtualMIDIClosePort", SetLastError = true)]
    internal static extern void ClosePort(IntPtr port);
    [System.Runtime.InteropServices.DllImport(DllName, EntryPoint = "virtualMIDIGetVersion", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
    private static extern IntPtr GetVersionNative(IntPtr major, IntPtr minor, IntPtr release, IntPtr build);

    internal static string GetVersion()
    {
        var version = GetVersionNative(IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
        return version == IntPtr.Zero ? "teVirtualMIDI" : Marshal.PtrToStringUni(version) ?? "teVirtualMIDI";
    }

    internal static string DescribeError(int code) => code switch
    {
        5 => "Access to the teVirtualMIDI driver was denied. Check the driver permissions for this Windows account.",
        3 => "The teVirtualMIDI driver is not installed or is inactive.",
        6 => "The teVirtualMIDI port is no longer available.",
        123 => "The MIDI port name contains an unsupported character.",
        126 => "The teVirtualMIDI interface DLL could not be loaded.",
        183 or 1379 => "A MIDI port with that name already exists.",
        1306 => "The teVirtualMIDI DLL and driver versions do not match.",
        1150 => "This Windows version is not supported by the installed teVirtualMIDI driver.",
        56 => "The MIDI command exceeds the driver buffer limit.",
        69 => "The teVirtualMIDI driver has reached its virtual port limit.",
        _ => $"teVirtualMIDI error {code}."
    };
}
