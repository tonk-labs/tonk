import React, {useState, useRef, useEffect} from 'react';
import {Bot, Settings} from 'lucide-react';
import styles from './Chat.module.css';

interface Content {
  value: string;
  isHuman: boolean;
  isThinking?: boolean;
  thinkingContent?: string;
}

interface ChatLogProps {
  content?: Content[];
  nav: (pageName: string) => void
}

type Message = {
  role: 'human' | 'assistant';
  content: string;
};

const ChatLog: React.FC<ChatLogProps> = (props: ChatLogProps) => {
  const [content, setContent] = useState<Content[]>([]);
  const [message, setMessage] = useState('');
  const [showPlaceholder, setShowPlaceholder] = useState(true);
  const inputRef = useRef<HTMLSpanElement | null>(null); // Create a ref for the span
  const [socket, setSocket] = useState<WebSocket | null>(null);
  const [isConnected, setIsConnected] = useState(false);

  // Set up WebSocket connection
  useEffect(() => {
    const ws = new WebSocket('ws://localhost:3000');
    
    ws.onopen = () => {
      console.log('Connected to WebSocket server');
      setIsConnected(true);
      setSocket(ws);
    };
    
    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        console.log('Received message from server:', data);
        
        // Add server response to chat
        if (data.message) {
          setContent(prev => [...prev, {
            value: data.message,
            isHuman: false
          }]);
        }
      } catch (error) {
        console.error('Error processing server message:', error);
      }
    };
    
    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      setIsConnected(false);
    };
    
    ws.onclose = () => {
      console.log('Disconnected from WebSocket server');
      setIsConnected(false);
      setSocket(null);
    };
    
    return () => {
      ws.close();
    };
  }, []);

  const setFocus = () => {
    inputRef.current?.focus(); // Set focus on the span when the component mounts
    setShowPlaceholder(false);
  };

  // Add a new function to handle input and maintain cursor position
  const handleInput = (e: React.FormEvent<HTMLSpanElement>) => {
    const target = e.currentTarget;
    const newMessage = target.textContent || '';
    setMessage(newMessage);
    setShowPlaceholder(false); // Hide placeholder when typing

    // Maintain cursor position
    const range = document.createRange();
    const sel = window.getSelection();
    range.setStart(target, target.childNodes.length);
    range.collapse(true);
    sel?.removeAllRanges();
    sel?.addRange(range);
  };

  const handleSubmit = async () => {
    if (!message.trim() || !socket || !isConnected) return;

    // Create message object
    const messageObj: Message = {
      role: 'human',
      content: message.trim()
    };

    // Add message to UI
    setContent(prev => [...prev, {
      value: message.trim(),
      isHuman: true
    }]);

    // Send message to server
    try {
      socket.send(JSON.stringify(messageObj));
      console.log('Message sent to server:', messageObj);
      
      // Clear input field
      setMessage('');
      if (inputRef.current) {
        inputRef.current.textContent = '';
      }
      setShowPlaceholder(true);
    } catch (error) {
      console.error('Error sending message:', error);
      
      // Add error message to chat
      setContent(prev => [...prev, {
        value: 'Error sending message. Please try again.',
        isHuman: false
      }]);
    }
  };

  // Add this new function to handle key press
  const handleKeyPress = (e: React.KeyboardEvent<HTMLSpanElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  return (
    <div className={styles.container}>
      <div className={styles.settingsButton} onClick={() => props.nav("Settings")}>
        <Settings size={20} />
      </div>

      <div className={styles.chatContainer}>
        {content.map((msg, index) => (
          <div key={index} className={styles.messageWrapper}>
            <div
              className={msg.isHuman ? styles.humanMessage : styles.botMessage}
            >
              {!msg.isHuman ? (
                <>
                  <div className={styles.botIconWrapper}/>
                  <Bot size={20} className={styles.botIcon} />
                  <div className={styles.botContent}>{msg.value}</div>
                </>
              ) : (
                msg.value
              )}
            </div>
          </div>
        ))}
      </div>
      <div className={styles.inputContainer}>
        {showPlaceholder ? (
          <div className={styles.placeholder} onClick={() => setFocus()}>
            Type in your message...
          </div>
        ) : (
          ''
        )}
        <span
          ref={inputRef}
          role="textbox"
          dir="ltr"
          contentEditable
          className={styles.inputField}
          onInput={handleInput}
          onKeyDown={handleKeyPress}
          onBlur={() => (message === '' ? setShowPlaceholder(true) : false)}
          onFocus={() => setShowPlaceholder(false)}
          inputMode="text"
          aria-label="Chat input"
        >
          {message}
        </span>
        <div
          className={`${styles.submitButton} ${showPlaceholder ? styles.hidden : styles.visible}`}
          onClick={handleSubmit}
        >
          Submit
        </div>
      </div>
      {!isConnected && (
        <div className={styles.disconnectedAlert}>
          Disconnected from server
        </div>
      )}
    </div>
  );
};

export default ChatLog;
