window.addEventListener('load', () => {
    const messages = document.getElementById('messages');
    const form = document.getElementById('form');
    const input = document.getElementById('input');
    const token = localStorage.getItem('chatToken'); // Retrieve token

    function addMessage(msg, type = 'normal') {
        const item = document.createElement('li');
        item.textContent = msg;
        if (type === 'info' || type === 'error' || type === 'private') {
             item.classList.add(type);
        }
        // Simple detection for private messages based on content
        if (msg.startsWith('(Private from') || msg.startsWith('(Message sent to') || msg.startsWith('(User')) {
            item.classList.add('private');
        } else if (msg.includes(' joined') || msg.includes(' left') || msg.startsWith('Welcome') || msg.startsWith('Online users:')) {
            item.classList.add('info');
        }
        messages.appendChild(item);
        messages.scrollTop = messages.scrollHeight; // Scroll to bottom
    }

    if (!token) {
        addMessage('Authentication token not found. Redirecting to login...', 'error');
        setTimeout(() => {
            window.location.href = '/'; // Redirect to login page (index.html)
        }, 2000);
        return; // Stop execution
    }

    // Construct WebSocket URL
    const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${wsProtocol}//${window.location.host}/ws/`; // Adjust if server runs elsewhere

    // --- Connect WebSocket with Token in Subprotocol ---
    let ws;
    try {
         // The browser sends the token in the Sec-WebSocket-Protocol header
        ws = new WebSocket(wsUrl, token); // Pass token as the subprotocol string
        addMessage('Connecting to chat server...', 'info');
    } catch (e) {
        addMessage(`Failed to create WebSocket: ${e}`, 'error');
         console.error("WebSocket creation error:", e);
        return;
    }


    ws.onopen = (event) => {
         console.log("WebSocket connection opened:", event);
        // Find and remove the initial "Connecting..." message
         const connectingMsg = Array.from(messages.children).find(li => li.textContent.includes('Connecting...'));
         if (connectingMsg) connectingMsg.remove();
         addMessage('Connected!', 'info');
        input.focus();
    };

    ws.onmessage = (event) => {
        console.log('Message from server:', event.data);
        addMessage(event.data);
    };

    ws.onerror = (event) => {
        console.error('WebSocket error:', event);
        addMessage('WebSocket error occurred. Connection might be lost.', 'error');
    };

    ws.onclose = (event) => {
        console.log('WebSocket connection closed:', event.code, event.reason);
        let message = `Disconnected: ${event.reason || 'Connection closed'}`;
        if (event.code !== 1000 && event.code !== 1005) { // 1000=Normal, 1005=No Status Rcvd
             message += ` (Code: ${event.code})`;
        }
         addMessage(message, 'error');
         // Optionally disable input or attempt reconnect
         input.disabled = true;
         form.querySelector('button').disabled = true;
    };

    form.addEventListener('submit', (event) => {
        event.preventDefault();
        if (input.value && ws && ws.readyState === WebSocket.OPEN) {
            console.log('Sending:', input.value)
            ws.send(input.value);
            input.value = ''; // Clear input
        } else {
             console.warn("Cannot send message, WebSocket not open or input empty.");
             if (!ws || ws.readyState !== WebSocket.OPEN) {
                 addMessage('Not connected to server.', 'error');
             }
        }
         input.focus();
    });

});
