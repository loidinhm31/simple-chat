let peerConnection;
let localStream;
let ws;
const userId = Math.random().toString(36).substring(7);
const room = 'default-room';

const config = {
    iceServers: [
        {
            urls: [
                'stun:stun1.l.google.com:19302',
                'stun:stun2.l.google.com:19302'
            ]
        }
    ]
};

// DOM Elements
const startButton = document.getElementById('startButton');
const audioOnlyButton = document.getElementById('audioOnlyButton');
const mainContainer = document.getElementById('mainContainer');
const errorDiv = document.getElementById('error');

startButton.addEventListener('click', () => initializeMedia(false));
audioOnlyButton.addEventListener('click', () => initializeMedia(true));

function showError(message) {
    errorDiv.textContent = message;
    errorDiv.classList.remove('hidden');
}

async function initializeMedia(audioOnly) {
    try {
        const constraints = {
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            },
            video: audioOnly ? false : {
                width: { ideal: 640 },
                height: { ideal: 480 },
                facingMode: "user"
            }
        };

        localStream = await navigator.mediaDevices.getUserMedia(constraints);
        const localVideo = document.getElementById('localVideo');
        if (localVideo) {
            localVideo.srcObject = localStream;
        }

        // Show main container and hide setup
        mainContainer.classList.remove('hidden');
        startButton.parentElement.classList.add('hidden');

        // Initialize WebSocket after media setup
        initializeWebSocket();
    } catch (e) {
        console.error('Media access error:', e);
        if (e.name === 'NotAllowedError') {
            showError('Permission denied. Please allow camera/microphone access and try again.');
        } else if (e.name === 'NotFoundError') {
            showError('No camera/microphone found. Please check your device.');
        } else {
            showError(`Error accessing media: ${e.message}`);
        }
    }
}

function initializeWebSocket() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/ws`;

    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
        console.log('WebSocket connected');
        sendToServer({
            event: 'join',
            room: room,
            from: userId,
            data: ''
        });
        createPeerConnection();
    };

    ws.onmessage = handleSignalingMessage;

    ws.onerror = (error) => {
        console.error('WebSocket error:', error);
        showError('Connection error. Please try refreshing the page.');
    };

    ws.onclose = () => {
        console.log('WebSocket closed');
        showError('Connection closed. Please refresh the page to reconnect.');
    };
}

function createPeerConnection() {
    try {
        peerConnection = new RTCPeerConnection(config);

        localStream.getTracks().forEach(track => {
            peerConnection.addTrack(track, localStream);
        });

        peerConnection.ontrack = (event) => {
            const remoteVideo = document.getElementById('remoteVideo');
            if (remoteVideo.srcObject !== event.streams[0]) {
                remoteVideo.srcObject = event.streams[0];
            }
        };

        peerConnection.onicecandidate = (event) => {
            if (event.candidate) {
                sendToServer({
                    event: 'ice-candidate',
                    data: JSON.stringify(event.candidate),
                    room: room,
                    from: userId
                });
            }
        };

        peerConnection.oniceconnectionstatechange = () => {
            console.log('ICE connection state:', peerConnection.iceConnectionState);
        };
    } catch (e) {
        console.error('Error creating peer connection:', e);
        showError('Error setting up video chat. Please refresh and try again.');
    }
}

// Add server camera canvas element
const serverCanvas = document.createElement('canvas');
serverCanvas.id = 'serverCanvas';
serverCanvas.style.width = '100%';
serverCanvas.style.backgroundColor = '#333';
serverCanvas.width = 320;  // Match camera resolution
serverCanvas.height = 240;
document.querySelector('.video-container').appendChild(serverCanvas);

async function handleSignalingMessage(message) {
    try {
        const msg = JSON.parse(message.data);
        console.log('Received message:', msg.event);

        switch (msg.event) {
            case 'user_joined':
                console.log('User joined:', msg.data);
                if (msg.from === 'server' && msg.data !== userId) {
                    console.log('Creating offer for new user');
                    await createOffer();
                }
                break;

            case 'user_left':
                console.log('User left:', msg.data);
                // Handle user disconnection if needed
                break;

            case 'offer':
                console.log('Received offer from:', msg.from);
                if (!msg.to || msg.to === userId) {
                    await handleOffer(msg);
                }
                break;

            case 'answer':
                console.log('Received answer from:', msg.from);
                if (msg.to === userId) {
                    const answer = JSON.parse(msg.data);
                    await peerConnection.setRemoteDescription(answer);
                }
                break;

            case 'ice-candidate':
                console.log('Received ICE candidate');
                if (!msg.to || msg.to === userId) {
                    const candidate = JSON.parse(msg.data);
                    await peerConnection.addIceCandidate(candidate);
                }
                break;

            case 'message':
                console.log('Received chat message from:', msg.from);
                if (msg.from !== userId) {
                    displayMessage(msg.data, false);
                }
                break;

            case 'camera-frame':
                if (msg.from === 'server-camera') {
                    const canvas = document.getElementById('serverCanvas');
                    if (canvas) {
                        const ctx = canvas.getContext('2d');
                        const img = new Image();
                        img.onload = () => {
                            ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
                        };
                        img.src = 'data:image/jpeg;base64,' + msg.data;
                    }
                }
                break;
        }
    } catch (e) {
        console.error('Error handling message:', e);
    }
}

async function createOffer() {
    try {
        const offer = await peerConnection.createOffer({
            offerToReceiveAudio: true,
            offerToReceiveVideo: true
        });
        await peerConnection.setLocalDescription(offer);

        sendToServer({
            event: 'offer',
            data: JSON.stringify(offer),
            room: room,
            from: userId,
            to: null
        });
    } catch (e) {
        console.error('Error creating offer:', e);
        showError('Error establishing connection. Please refresh and try again.');
    }
}

async function handleOffer(msg) {
    try {
        await peerConnection.setRemoteDescription(JSON.parse(msg.data));
        const answer = await peerConnection.createAnswer();
        await peerConnection.setLocalDescription(answer);

        sendToServer({
            event: 'answer',
            data: JSON.stringify(answer),
            room: room,
            from: userId,
            to: msg.from
        });
    } catch (e) {
        console.error('Error handling offer:', e);
        showError('Error connecting to peer. Please refresh and try again.');
    }
}

function sendToServer(message) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(message));
    }
}

function displayMessage(message, isSelf) {
    const messageDiv = document.createElement('div');
    messageDiv.className = `message ${isSelf ? 'self' : 'other'}`;
    messageDiv.textContent = message;
    document.getElementById('messages').appendChild(messageDiv);
    messageDiv.scrollIntoView({ behavior: 'smooth' });
}

function sendMessage() {
    const input = document.getElementById('messageInput');
    const message = input.value.trim();

    if (message) {
        displayMessage(message, true);
        input.value = '';

        sendToServer({
            event: 'message',
            data: message,
            room: room,
            from: userId
        });
    }
}