import requests
import json

BASE_URL = "http://localhost:8080"
API_KEY = "default_api_key"
HEADERS = {
    "Content-Type": "application/json",
    "x-api-key": API_KEY
}

def test_get_provider_config():
    print("\n--- Testing GET /provider/config ---")
    url = f"{BASE_URL}/provider/config"
    response = requests.get(url, headers={"x-api-key": API_KEY})
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "provider" in response.json()
    assert "model" in response.json()

def test_start_session():
    print("\n--- Testing POST /session/start ---")
    url = f"{BASE_URL}/session/start"
    data = {"prompt": "Create a Python function to generate Fibonacci numbers"}
    response = requests.post(url, headers=HEADERS, data=json.dumps(data))
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "session_id" in response.json()
    return response.json().get("session_id")

def test_reply_session(session_id):
    print(f"\n--- Testing POST /session/reply for session_id: {session_id} ---")
    url = f"{BASE_URL}/session/reply"
    data = {"session_id": session_id, "prompt": "Continue with the next Fibonacci number."}
    response = requests.post(url, headers=HEADERS, data=json.dumps(data))
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "message" in response.json()

def test_summarize_session(session_id):
    print(f"\n--- Testing POST /session/summarize for session_id: {session_id} ---")
    url = f"{BASE_URL}/session/summarize"
    data = {"session_id": session_id}
    response = requests.post(url, headers=HEADERS, data=json.dumps(data))
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "summary" in response.json()

def test_end_session(session_id):
    print(f"\n--- Testing POST /session/end for session_id: {session_id} ---")
    url = f"{BASE_URL}/session/end"
    data = {"session_id": session_id}
    response = requests.post(url, headers=HEADERS, data=json.dumps(data))
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "message" in response.json()

def test_list_extensions():
    print("\n--- Testing GET /extensions/list ---")
    url = f"{BASE_URL}/extensions/list"
    response = requests.get(url, headers=HEADERS) # API key is not enforced for this endpoint, but including for consistency
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "extensions" in response.json()

def test_get_metrics():
    print("\n--- Testing GET /metrics ---")
    url = f"{BASE_URL}/metrics"
    response = requests.get(url) # No API key required
    print(f"Status Code: {response.status_code}")
    print(f"Response: {response.json()}")
    assert response.status_code == 200
    assert "active_sessions" in response.json()
    assert "pending_requests" in response.json()

if __name__ == "__main__":
    print("Starting API endpoint tests...")
    
    # Test endpoints that don't require a session_id first
    test_get_provider_config()
    test_list_extensions()
    test_get_metrics()

    # Test session-related endpoints
    session_id = test_start_session()
    if session_id:
        test_reply_session(session_id)
        test_summarize_session(session_id)
        test_end_session(session_id)
    else:
        print("Skipping session tests as session_id was not obtained.")

    print("\nAll tests completed.")